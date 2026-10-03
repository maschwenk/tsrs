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
        // Debug aid: TSRS_TEST_BACKTRACE=<file> appends every panic's backtrace to <file>.
        if let Some(path) = std::env::var_os("TSRS_TEST_BACKTRACE") {
            let trace = format!("panic: {message} at {location}\n{}\n", std::backtrace::Backtrace::force_capture());
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = f.write_all(trace.as_bytes());
            }
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
    // `.types` / `.symbols` (only when requested with --baselines).
    pub types: Option<ExtraResult>,
    pub symbols: Option<ExtraResult>,
    pub js: Option<ExtraResult>,
}

pub struct ExtraResult {
    pub class: Class,
    // First difference, panic message or skip reason.
    pub diff: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

impl ExtraResult {
    fn plain(class: Class, diff: String) -> ExtraResult {
        ExtraResult { class, diff, expected: None, actual: None }
    }

    fn classify(item: &TestItem, ext: &str, generated: &Result<String, String>) -> ExtraResult {
        match generated {
            Err(panic) => ExtraResult::plain(Class::Crash, format!("panic: {panic}")),
            Ok(actual) => {
                let expected = compiler_runner::read_reference_extra_baseline(&item.suite, &item.name, ext);
                let class = if baseline::baseline_matches(expected.as_deref(), actual) { Class::Pass } else { Class::Fail };
                let diff = if class == Class::Pass { String::new() } else { baseline::first_difference(expected.as_deref(), actual) };
                ExtraResult { class, diff, expected, actual: Some(actual.clone()) }
            }
        }
    }
}

pub const EXTRA_KINDS: [(&str, u8); 3] = [("types", crate::EXTRA_TYPES), ("symbols", crate::EXTRA_SYMBOLS), ("js", crate::EXTRA_JS)];

pub fn results_dir() -> PathBuf {
    match std::env::var_os("TSRS_TEST_RESULTS") {
        Some(p) => PathBuf::from(p),
        None if crate::syntax_only() => compiler_runner::repo_root().join("target/test-results-syntax"),
        None => compiler_runner::repo_root().join("target/test-results"),
    }
}

pub fn run_item(backend: &Backend, item: &TestItem) -> ItemResult {
    let start = Instant::now();
    LAST_PANIC.with(|p| *p.borrow_mut() = None);
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| backend.run(item)));
    let ms = start.elapsed().as_millis();
    let mut r = ItemResult {
        class: Class::Fail,
        ms,
        diff: String::new(),
        panic: None,
        skip: String::new(),
        expected: None,
        actual: None,
        types: None,
        symbols: None,
        js: None,
    };
    let want = crate::extra_baselines();
    let mut extras: Vec<(&str, ExtraResult)> = Vec::new();
    match outcome {
        Err(_) => {
            r.class = Class::Crash;
            r.panic = Some(LAST_PANIC.with(|p| p.borrow_mut().take()).unwrap_or(PanicInfo { message: "<unknown panic>".into(), location: String::new() }));
            for (ext, _) in EXTRA_KINDS {
                extras.push((ext, ExtraResult::plain(Class::Crash, "errors phase panicked".to_string())));
            }
        }
        Ok(Outcome::Skip(reason)) => {
            r.class = Class::Skip;
            for (ext, _) in EXTRA_KINDS {
                extras.push((ext, ExtraResult::plain(Class::Skip, reason.clone())));
            }
            r.skip = reason;
        }
        Ok(Outcome::Error(e)) => {
            r.class = Class::Fail;
            r.diff = format!("harness error: {e}");
            for (ext, _) in EXTRA_KINDS {
                extras.push((ext, ExtraResult::plain(Class::Fail, r.diff.clone())));
            }
        }
        Ok(Outcome::Baseline(actual, types_and_symbols, js)) => {
            match &js {
                None => extras.push(("js", ExtraResult::plain(Class::Skip, "no js output".to_string()))),
                Some(js) => extras.push(("js", ExtraResult::classify(item, "js", js))),
            }
            match &types_and_symbols {
                None => {
                    for ext in ["types", "symbols"] {
                        extras.push((ext, ExtraResult::plain(Class::Skip, "noTypesAndSymbols".to_string())));
                    }
                }
                Some(t) => {
                    extras.push(("types", ExtraResult::classify(item, "types", &t.types)));
                    extras.push(("symbols", ExtraResult::classify(item, "symbols", &t.symbols)));
                }
            }
            let expected = compiler_runner::read_reference_baseline(&item.suite, &item.name);
            r.class = baseline::classify(expected.as_deref(), &actual);
            if r.class != Class::Pass {
                r.diff = baseline::first_difference(expected.as_deref(), &actual);
            }
            r.expected = expected;
            r.actual = Some(actual);
        }
    }
    for (ext, e) in extras {
        match ext {
            "types" if want & crate::EXTRA_TYPES != 0 => r.types = Some(e),
            "symbols" if want & crate::EXTRA_SYMBOLS != 0 => r.symbols = Some(e),
            "js" if want & crate::EXTRA_JS != 0 => r.js = Some(e),
            _ => {}
        }
    }
    r
}

pub fn extra_result<'a>(r: &'a ItemResult, ext: &str) -> Option<&'a ExtraResult> {
    match ext {
        "types" => r.types.as_ref(),
        "js" => r.js.as_ref(),
        _ => r.symbols.as_ref(),
    }
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

// Writes (or removes stale) `<suite>/<name>.{types,symbols}.{actual,diff}` for the requested extra baselines.
pub fn write_extra_artifacts(item: &TestItem, r: &ItemResult) {
    let dir = results_dir().join(&item.suite);
    for (ext, bit) in EXTRA_KINDS {
        if crate::extra_baselines() & bit == 0 {
            continue;
        }
        let actual_path = dir.join(format!("{}.{ext}.actual", item.name));
        let diff_path = dir.join(format!("{}.{ext}.diff", item.name));
        let _ = std::fs::remove_file(&actual_path);
        let _ = std::fs::remove_file(&diff_path);
        let Some(e) = extra_result(r, ext) else { continue };
        if e.class == Class::Pass || e.class == Class::Skip {
            continue;
        }
        let _ = std::fs::create_dir_all(&dir);
        if let Some(actual) = &e.actual {
            let _ = std::fs::write(&actual_path, actual);
            let _ = std::fs::write(&diff_path, baseline::unified_diff(e.expected.as_deref(), actual, &format!("{}.{ext}", item.name)));
        } else if !e.diff.is_empty() {
            let _ = std::fs::write(&diff_path, format!("{}\n", e.diff));
        }
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
                write_extra_artifacts(&item, &r);
                let mut v = json!({"i": i, "class": r.class.as_str(), "ms": r.ms as u64});
                for (ext, _) in EXTRA_KINDS {
                    if let Some(e) = extra_result(&r, ext) {
                        v[ext] = json!(e.class.as_str());
                        if !e.diff.is_empty() {
                            v[format!("{ext}_diff")] = json!(e.diff);
                        }
                    }
                }
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

/// The message of the last panic caught on this thread (for panics caught inside a test, like Go's RecoverAndFail).
pub fn take_last_panic_message() -> String {
    LAST_PANIC.with(|p| p.borrow_mut().take()).map(|p| format!("{} at {}", p.message, p.location)).unwrap_or_else(|| "<unknown panic>".to_string())
}
