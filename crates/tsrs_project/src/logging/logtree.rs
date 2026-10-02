use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::SystemTime;

use super::logcollector::LogCollector;
use super::logger::{format_time, Logger};

static seq: AtomicU64 = AtomicU64::new(0);

struct logEntry {
    seq: u64,
    time: SystemTime,
    message: String,
    child: LogTree,
}

// logtree.go:20
fn new_log_entry(child: LogTree, message: String) -> logEntry {
    logEntry { seq: seq.fetch_add(1, Ordering::Relaxed) + 1, time: SystemTime::now(), message, child }
}

// Go `*LogTree`; the default value is the nil tree, on which every method is a no-op (Go's nil receiver checks).
#[derive(Clone, Default)]
pub struct LogTree(Option<Arc<logTreeNode>>);

struct logTreeNode {
    name: String,
    logs: Mutex<Vec<logEntry>>,
    // None on the root itself (Go: `root == self`).
    root: Option<Weak<logTreeNode>>,
    level: i32,
    verbose: AtomicBool,

    // Only set on root
    count: AtomicI32,
    string_length: AtomicI32,
}

// logtree.go:44
pub fn new_log_tree(name: &str) -> LogTree {
    LogTree(Some(Arc::new(logTreeNode {
        name: name.to_string(),
        logs: Mutex::new(Vec::new()),
        root: None,
        level: 0,
        verbose: AtomicBool::new(false),
        count: AtomicI32::new(0),
        string_length: AtomicI32::new(0),
    })))
}

impl logTreeNode {
    fn with_root<R>(self: &Arc<Self>, f: impl FnOnce(&logTreeNode) -> R) -> R {
        match &self.root {
            None => f(self),
            Some(root) => match root.upgrade() {
                Some(root) => f(&root),
                None => f(self),
            },
        }
    }

    // logtree.go:52
    fn add(self: &Arc<Self>, log: logEntry) {
        // indent + header + message + newline
        let len = self.level + 15 + log.message.len() as i32 + 1;
        self.with_root(|root| {
            root.string_length.fetch_add(len, Ordering::Relaxed);
            root.count.fetch_add(1, Ordering::Relaxed);
        });
        self.logs.lock().unwrap().push(log);
    }

    // logtree.go:152
    fn write_logs_recursive(&self, builder: &mut String, indent: &str) {
        for log in self.logs.lock().unwrap().iter() {
            builder.push_str(indent);
            builder.push_str(&format_time(log.time));
            builder.push(' ');
            builder.push_str(&log.message);
            builder.push('\n');
            if let Some(child) = &log.child.0 {
                child.write_logs_recursive(builder, &format!("{indent}\t"));
            }
        }
    }
}

impl LogTree {
    pub fn nil() -> LogTree {
        LogTree(None)
    }

    pub fn is_nil(&self) -> bool {
        self.0.is_none()
    }

    // logtree.go:61
    pub fn log(&self, message: &str) {
        let Some(c) = &self.0 else {
            return;
        };
        c.add(new_log_entry(LogTree::nil(), message.to_string()));
    }

    // logtree.go:69
    pub fn logf(&self, args: fmt::Arguments<'_>) {
        let Some(c) = &self.0 else {
            return;
        };
        c.add(new_log_entry(LogTree::nil(), args.to_string()));
    }

    // logtree.go:77
    pub fn is_verbose(&self) -> bool {
        self.0.as_ref().is_some_and(|c| c.verbose.load(Ordering::Relaxed))
    }

    // logtree.go:81
    pub fn set_verbose(&self, verbose: bool) {
        if let Some(c) = &self.0 {
            c.verbose.store(verbose, Ordering::Relaxed);
        }
    }

    // logtree.go:88
    pub fn verbose(&self) -> LogTree {
        match &self.0 {
            Some(c) if c.verbose.load(Ordering::Relaxed) => self.clone(),
            _ => LogTree::nil(),
        }
    }

    // logtree.go:119
    pub fn embed(&self, logs: &LogTree) {
        let Some(c) = &self.0 else {
            return;
        };
        // Go dereferences a nil `logs` here; there is nothing to embed.
        let Some(l) = &logs.0 else {
            return;
        };
        let count = l.count.load(Ordering::Relaxed);
        let len = l.string_length.load(Ordering::Relaxed) + count * c.level;
        c.with_root(|root| {
            root.string_length.fetch_add(len, Ordering::Relaxed);
            root.count.fetch_add(count, Ordering::Relaxed);
        });
        c.add(new_log_entry(logs.clone(), l.name.clone()));
    }

    // logtree.go:130
    pub fn fork(&self, message: &str) -> LogTree {
        let Some(c) = &self.0 else {
            return LogTree::nil();
        };
        let root = match &c.root {
            None => Arc::downgrade(c),
            Some(root) => root.clone(),
        };
        let child = LogTree(Some(Arc::new(logTreeNode {
            name: String::new(),
            logs: Mutex::new(Vec::new()),
            root: Some(root),
            level: c.level + 1,
            verbose: AtomicBool::new(c.verbose.load(Ordering::Relaxed)),
            count: AtomicI32::new(0),
            string_length: AtomicI32::new(0),
        })));
        c.add(new_log_entry(child.clone(), message.to_string()));
        child
    }

    // logtree.go:140
    pub fn string(&self) -> String {
        let c = self.0.as_ref().expect("String called on nil LogTree");
        if c.root.is_some() {
            panic!("can only call String on root LogTree");
        }
        let header = format!("======== {} ========\n", c.name);
        let mut builder = String::with_capacity(c.string_length.load(Ordering::Relaxed).max(0) as usize + header.len());
        builder.push_str(&header);
        c.write_logs_recursive(&mut builder, "");
        builder
    }
}

impl Logger for LogTree {
    // logtree.go:95
    fn error(&self, msg: &str) {
        self.log(msg)
    }
    // logtree.go:99
    fn errorf(&self, args: fmt::Arguments<'_>) {
        self.logf(args)
    }
    // logtree.go:103
    fn warn(&self, msg: &str) {
        self.log(msg)
    }
    // logtree.go:107
    fn warnf(&self, args: fmt::Arguments<'_>) {
        self.logf(args)
    }
    // logtree.go:111
    fn info(&self, msg: &str) {
        self.log(msg)
    }
    // logtree.go:115
    fn infof(&self, args: fmt::Arguments<'_>) {
        self.logf(args)
    }
    fn log(&self, msg: &str) {
        LogTree::log(self, msg)
    }
    fn logf(&self, args: fmt::Arguments<'_>) {
        LogTree::logf(self, args)
    }
    fn verbose(&self) -> Option<&dyn Logger> {
        if self.is_nil() || !LogTree::is_verbose(self) {
            return None;
        }
        Some(self)
    }
    fn is_verbose(&self) -> bool {
        LogTree::is_verbose(self)
    }
    fn set_verbose(&self, verbose: bool) {
        LogTree::set_verbose(self, verbose)
    }
}

impl LogCollector for LogTree {
    fn string(&self) -> String {
        LogTree::string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // logtree_test.go:12 TestLogTreeImplementsLogger
    #[test]
    fn log_tree_implements_logger() {
        let tree = new_log_tree("test");
        let _: &dyn Logger = &tree;
    }

    // logtree_test.go:17 TestLogTree (empty in Go); tsrs-only: the rendering of nested forks.
    #[test]
    fn log_tree() {
        let tree = new_log_tree("root");
        tree.log("a");
        let child = tree.fork("child");
        child.log("b");
        let s = tree.string();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines[0], "======== root ========");
        assert!(lines[1].ends_with(" a"));
        assert!(lines[2].ends_with(" child"));
        assert!(lines[3].starts_with('\t') && lines[3].ends_with(" b"));
        LogTree::nil().fork("x").log("ignored");
    }
}
