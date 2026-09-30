// Worker side of the process pool: runs test items sequentially on a big-stack thread, catching panics,
// and streams one JSON result line per item to stdout.

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::time::Instant;

use serde_json::json;

use crate::baseline::{self, Class};
use crate::compiler_runner::{self, Outcome, TestItem};
use crate::Backend;

pub const WORKER_STACK_SIZE: usize = 256 << 20;

thread_local! {
    static LAST_PANIC: RefCell<Option<PanicInfo>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug)]
pub struct PanicInfo {
    pub message: String,
    pub location: String,
}

pub fn install_panic_hook(quiet: bool) {
    panic::set_hook(Box::new(move |info| {
        let message = info.payload_as_str().unwrap_or("<non-string panic payload>").to_string();
        let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        if !quiet {
            eprintln!("panic: {message} at {location}");
        }
        LAST_PANIC.with(|p| *p.borrow_mut() = Some(PanicInfo { message, location }));
    }));
}

pub struct ItemResult {
    pub class: Class,
    pub ms: u128,
    pub diff: String,
    pub panic: Option<PanicInfo>,
    pub skip: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

pub fn results_dir() -> PathBuf {
    match std::env::var_os("TSRS_TEST_RESULTS") {
        Some(p) => PathBuf::from(p),
        None => compiler_runner::repo_root().join("target/test-results"),
    }
}

pub fn run_item(backend: &Backend, item: &TestItem) -> ItemResult {
    let start = Instant::now();
    LAST_PANIC.with(|p| *p.borrow_mut() = None);
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| backend.run(item)));
    let ms = start.elapsed().as_millis();
    let mut r = ItemResult { class: Class::Fail, ms, diff: String::new(), panic: None, skip: String::new(), expected: None, actual: None };
    match outcome {
        Err(_) => {
            r.class = Class::Crash;
            r.panic = Some(LAST_PANIC.with(|p| p.borrow_mut().take()).unwrap_or(PanicInfo { message: "<unknown panic>".into(), location: String::new() }));
        }
        Ok(Outcome::Skip(reason)) => {
            r.class = Class::Skip;
            r.skip = reason;
        }
        Ok(Outcome::Error(e)) => {
            r.class = Class::Fail;
            r.diff = format!("harness error: {e}");
        }
        Ok(Outcome::Baseline(actual)) => {
            let expected = compiler_runner::read_reference_baseline(&item.suite, &item.name);
            r.class = baseline::classify(expected.as_deref(), &actual);
            if r.class != Class::Pass {
                r.diff = baseline::first_difference(expected.as_deref(), &actual);
            }
            r.expected = expected;
            r.actual = Some(actual);
        }
    }
    r
}

// Writes (or removes stale) `<suite>/<name>.{actual.txt,diff}` for one result.
pub fn write_artifacts(item: &TestItem, r: &ItemResult) {
    let dir = results_dir().join(&item.suite);
    let actual_path = dir.join(format!("{}.actual.txt", item.name));
    let diff_path = dir.join(format!("{}.diff", item.name));
    let _ = std::fs::remove_file(&actual_path);
    let _ = std::fs::remove_file(&diff_path);
    if r.class == Class::Pass || r.class == Class::Skip {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    if let Some(actual) = &r.actual {
        let _ = std::fs::write(&actual_path, actual);
        let _ = std::fs::write(&diff_path, baseline::unified_diff(r.expected.as_deref(), actual, &format!("{}.errors.txt", item.name)));
    } else if let Some(p) = &r.panic {
        let _ = std::fs::write(&diff_path, format!("panic: {}\n  at {}\n", p.message, p.location));
    } else if !r.diff.is_empty() {
        let _ = std::fs::write(&diff_path, format!("{}\n", r.diff));
    }
}

pub fn parse_item_line(line: &str) -> TestItem {
    let mut parts = line.split('\t');
    let mut next = || parts.next().unwrap_or("").to_string();
    TestItem { suite: next(), path: next(), config: next(), name: next() }
}

pub fn format_item_line(item: &TestItem) -> String {
    format!("{}\t{}\t{}\t{}", item.suite, item.path, item.config, item.name)
}

// `tsrs-test __worker`: items arrive on stdin, one per line; results leave on stdout in the same order.
pub fn worker_main(backend_spec: crate::BackendSpec) {
    install_panic_hook(true);
    let handle = std::thread::Builder::new()
        .name("tsrs-test-worker".into())
        .stack_size(WORKER_STACK_SIZE)
        .spawn(move || {
            let backend = Backend::new(&backend_spec);
            let stdin = std::io::stdin();
            let mut stdout = std::io::stdout().lock();
            for (i, line) in stdin.lock().lines().enumerate() {
                let Ok(line) = line else { break };
                if line.is_empty() {
                    continue;
                }
                let item = parse_item_line(&line);
                let _ = writeln!(stdout, "{}", json!({"start": i}));
                let _ = stdout.flush();
                let r = run_item(&backend, &item);
                write_artifacts(&item, &r);
                let mut v = json!({"i": i, "class": r.class.as_str(), "ms": r.ms as u64});
                if !r.diff.is_empty() {
                    v["diff"] = json!(r.diff);
                }
                if !r.skip.is_empty() {
                    v["skip"] = json!(r.skip);
                }
                if let Some(p) = &r.panic {
                    v["panic"] = json!(p.message);
                    v["loc"] = json!(p.location);
                }
                let _ = writeln!(stdout, "{v}");
                let _ = stdout.flush();
            }
        })
        .unwrap();
    let _ = handle.join();
}
