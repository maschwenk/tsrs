// summary.json, per-class name lists, the summary table, and crash grouping.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::baseline::Class;
use crate::compiler_runner::SUITES;
use crate::pool::TestResult;

pub struct Entry {
    pub class: Class,
    pub ms: u64,
    pub diff: String,
    pub panic: String,
    pub loc: String,
    pub skip: String,
}

impl From<&TestResult> for Entry {
    fn from(r: &TestResult) -> Entry {
        Entry { class: r.class, ms: r.ms, diff: r.diff.clone(), panic: r.panic.clone(), loc: r.loc.clone(), skip: r.skip.clone() }
    }
}

pub type Summary = BTreeMap<String, Entry>;

pub fn load_summary(path: &Path) -> Summary {
    let mut summary = Summary::new();
    let Ok(text) = std::fs::read_to_string(path) else { return summary };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return summary };
    for t in v["tests"].as_array().into_iter().flatten() {
        let s = |k: &str| t[k].as_str().unwrap_or("").to_string();
        let Some(class) = Class::parse(t["class"].as_str().unwrap_or("")) else { continue };
        summary.insert(s("id"), Entry { class, ms: t["ms"].as_u64().unwrap_or(0), diff: s("diff"), panic: s("panic"), loc: s("loc"), skip: s("skip") });
    }
    summary
}

fn suite_of(id: &str) -> &str {
    id.split('/').next().unwrap_or("")
}

pub type Totals = BTreeMap<String, BTreeMap<Class, usize>>;

pub fn totals(summary: &Summary) -> Totals {
    let mut t: Totals = BTreeMap::new();
    for (id, e) in summary {
        *t.entry(suite_of(id).to_string()).or_default().entry(e.class).or_default() += 1;
        *t.entry("all".to_string()).or_default().entry(e.class).or_default() += 1;
    }
    t
}

pub fn write_summary(dir: &Path, json_path: &Path, summary: &Summary) {
    let _ = std::fs::create_dir_all(dir);
    let tots = totals(summary);
    let mut totals_json = serde_json::Map::new();
    for (suite, counts) in &tots {
        let mut m = serde_json::Map::new();
        for c in Class::ALL {
            m.insert(c.as_str().to_string(), json!(counts.get(&c).copied().unwrap_or(0)));
        }
        totals_json.insert(suite.clone(), Value::Object(m));
    }
    let tests: Vec<Value> = summary
        .iter()
        .map(|(id, e)| {
            let mut v = json!({"id": id, "class": e.class.as_str(), "ms": e.ms});
            for (k, s) in [("diff", &e.diff), ("panic", &e.panic), ("loc", &e.loc), ("skip", &e.skip)] {
                if !s.is_empty() {
                    v[k] = json!(s);
                }
            }
            v
        })
        .collect();
    let doc = json!({"totals": totals_json, "tests": tests});
    if let Some(parent) = json_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(json_path, serde_json::to_string_pretty(&doc).unwrap());
    for c in Class::ALL {
        let mut s = String::new();
        for (id, e) in summary {
            if e.class == c {
                s.push_str(id);
                s.push('\n');
            }
        }
        let _ = std::fs::write(dir.join(format!("{}.txt", c.as_str())), s);
    }
}

pub fn print_table(summary: &Summary) {
    let tots = totals(summary);
    print!("{:<12} {:>6}", "suite", "total");
    for c in Class::ALL {
        print!(" {:>7}", c.as_str());
    }
    println!();
    let mut rows: Vec<&str> = SUITES.iter().copied().filter(|s| tots.contains_key(*s)).collect();
    rows.push("all");
    for suite in rows {
        let counts = tots.get(suite).cloned().unwrap_or_default();
        let total: usize = counts.values().sum();
        print!("{suite:<12} {total:>6}");
        for c in Class::ALL {
            print!(" {:>7}", counts.get(&c).copied().unwrap_or(0));
        }
        println!();
    }
}

fn is_todo(message: &str) -> bool {
    message.starts_with("not yet implemented") || message.starts_with("not implemented")
}

// The Rust function enclosing `file:line:col`, found by scanning the source upward for `fn name`.
fn enclosing_fn(loc: &str) -> Option<String> {
    let mut parts = loc.rsplitn(3, ':');
    let _col = parts.next()?;
    let line: usize = parts.next()?.parse().ok()?;
    let file = parts.next()?;
    let path = if Path::new(file).is_absolute() { Path::new(file).to_path_buf() } else { crate::compiler_runner::repo_root().join(file) };
    let text = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    for l in lines[..line.min(lines.len())].iter().rev() {
        let t = l.trim_start();
        if let Some(idx) = t.find("fn ") {
            let prefix = &t[..idx];
            if prefix.is_empty() || prefix.ends_with("pub ") || prefix.ends_with(") ") || prefix.ends_with("unsafe ") || prefix.ends_with("const ") {
                let name: String = t[idx + 3..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

pub fn crash_key(e: &Entry) -> String {
    let short_loc = e.loc.split("/crates/").last().unwrap_or(&e.loc).to_string();
    let short_loc = if e.loc.contains("/crates/") { format!("crates/{short_loc}") } else { short_loc };
    if is_todo(&e.panic) {
        let f = enclosing_fn(&e.loc).map(|f| format!(" (fn {f})")).unwrap_or_default();
        return format!("todo at {short_loc}{f}");
    }
    let mut msg: String = e.panic.chars().take(160).collect();
    if e.panic.chars().count() > 160 {
        msg.push('…');
    }
    if short_loc.is_empty() {
        msg
    } else {
        format!("{msg} @ {short_loc}")
    }
}

pub fn print_crashes(summary: &Summary, top: usize, examples: usize) {
    let mut groups: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (id, e) in summary {
        if e.class == Class::Crash {
            groups.entry(crash_key(e)).or_default().push(id);
        }
    }
    let total: usize = groups.values().map(Vec::len).sum();
    let mut sorted: Vec<_> = groups.into_iter().collect();
    sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    println!("{total} crashes in {} groups", sorted.len());
    for (key, ids) in sorted.iter().take(top) {
        let ex: Vec<&str> = ids.iter().take(examples).copied().collect();
        println!("{:>6}  {key}", ids.len());
        if examples > 0 {
            println!("        e.g. {}", ex.join(", "));
        }
    }
}
