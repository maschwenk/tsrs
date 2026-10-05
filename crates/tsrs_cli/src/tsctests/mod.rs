// Port of Go's execute/tsctests runner (runner.go) for the non-watch tsc/tsbuild scenarios.
//
// The scenario tables of tsc_test.go / tscbuild_test.go are Go closures; tools/oracle/tsctests/dump.sh runs the Go
// tests once with a recorder and writes every scenario as JSON (inputs, and for every edit the file system changes it
// made). This harness replays them through `execute::command_line_with_testing` and compares the baseline with
// ts-ref/tsc/testdata/baselines/reference/<tsc|tsbuild>/<scenario>/<name>.js.
//
//   tools/oracle/tsctests/dump.sh                      # once (needs Go), writes target/tsctests-dump
//   cargo test --release -p tsrs_cli tsctests -- --nocapture
//   TSCTESTS_FILTER=<substring> ...                    # only matching baselines
//
// Results: target/tsctests-results/{pass,fail,crash}.txt and the actual baselines of failures under
// target/tsctests-results/local/. Content-mapper scenarios (not supported by tsrs) are skipped.

mod readablebuildinfo;
mod sys;

use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_core::json::Value;
use tsrs_core::tspath;
use tsrs_vfs::vfstest::{self, MapFile};
use tsrs_vfs::FS;

use crate::tsc::{CommandLineTesting, ExitStatus};
use sys::{TestClock, TestSys};

const TSC_LIB_PATH: &str = "/home/src/tslibs/TS/Lib";

struct scenarioFile {
    path: String,
    content: String,
    symlink: String,
}

struct scenarioOp {
    op: String,
    path: String,
    content: String,
    target: String,
}

struct scenarioEdit {
    caption: String,
    args: Option<Vec<String>>,
    expected_diff: String,
    ops: Vec<scenarioOp>,
    fresh_ops: Vec<scenarioOp>,
}

struct scenario {
    baseline: String,
    args: Vec<String>,
    cwd: String,
    env: FxHashMap<String, String>,
    output_is_tty: Option<bool>,
    ignore_case: bool,
    windows_style_root: String,
    files: Vec<scenarioFile>,
    default_libs: Vec<String>,
    edits: Vec<scenarioEdit>,
}

fn s(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn strings(v: Option<&Value>) -> Option<Vec<String>> {
    match v {
        Some(Value::Array(a)) => Some(a.iter().map(|x| s(Some(x))).collect()),
        _ => None,
    }
}

fn ops(v: Option<&Value>) -> Vec<scenarioOp> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .map(|o| {
                let Value::Object(o) = o else { panic!("op") };
                scenarioOp { op: s(o.get("op")), path: s(o.get("path")), content: s(o.get("content")), target: s(o.get("target")) }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn load_scenario(path: &FsPath) -> scenario {
    let text = std::fs::read_to_string(path).unwrap();
    let Value::Object(o) = tsrs_core::json::unmarshal(&text).unwrap() else { panic!("scenario") };
    let mut env = FxHashMap::default();
    if let Some(Value::Object(e)) = o.get("env") {
        for (k, v) in e {
            env.insert(k.clone(), s(Some(v)));
        }
    }
    let files = match o.get("files") {
        Some(Value::Array(a)) => a
            .iter()
            .map(|f| {
                let Value::Object(f) = f else { panic!("file") };
                scenarioFile { path: s(f.get("path")), content: s(f.get("content")), symlink: s(f.get("symlink")) }
            })
            .collect(),
        _ => Vec::new(),
    };
    let edits = match o.get("edits") {
        Some(Value::Array(a)) => a
            .iter()
            .map(|e| {
                let Value::Object(e) = e else { panic!("edit") };
                scenarioEdit {
                    caption: s(e.get("caption")),
                    args: strings(e.get("args")),
                    expected_diff: s(e.get("expectedDiff")),
                    ops: ops(e.get("ops")),
                    fresh_ops: ops(e.get("freshOps")),
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    scenario {
        baseline: s(o.get("baseline")),
        args: strings(o.get("args")).unwrap_or_default(),
        cwd: s(o.get("cwd")),
        env,
        output_is_tty: match o.get("outputIsTTY") {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        },
        ignore_case: matches!(o.get("ignoreCase"), Some(Value::Bool(true))),
        windows_style_root: s(o.get("windowsStyleRoot")),
        files,
        default_libs: strings(o.get("defaultLibs")).unwrap_or_default(),
        edits,
    }
}

// sys.go newTestSys: the initial file system comes from the dump (it already contains the default libraries,
// written after the test's own files like ensureLibPathExists does).
fn new_test_sys(test: &scenario) -> &'static TestSys {
    let clock = TestClock::new();
    let libs: FxHashSet<String> = test.default_libs.iter().cloned().collect();
    let mut initial: Vec<(String, MapFile)> = Vec::new();
    for f in &test.files {
        if libs.contains(&f.path) {
            continue;
        }
        if !f.symlink.is_empty() {
            initial.push((f.path.clone(), vfstest::symlink(&f.symlink)));
        } else {
            initial.push((f.path.clone(), MapFile::from(f.content.as_str())));
        }
    }
    let clock_fn = clock.clone();
    let fs = Arc::new(vfstest::from_map_with_clock(initial, !test.ignore_case, Arc::new(move || clock_fn.now())));
    let mut lib_files: Vec<&scenarioFile> = test.files.iter().filter(|f| libs.contains(&f.path)).collect();
    lib_files.sort_by(|a, b| a.path.cmp(&b.path));
    for f in lib_files {
        fs.write_file(&f.path, &f.content).unwrap_or_else(|e| panic!("Failed to write default library file: {e}"));
    }
    let lib_path = if test.windows_style_root.is_empty() { TSC_LIB_PATH.to_string() } else { format!("{}{}", test.windows_style_root, &TSC_LIB_PATH[1..]) };
    let cwd = if test.cwd.is_empty() { "/home/src/workspaces/project".to_string() } else { test.cwd.clone() };
    Box::leak(Box::new(TestSys::new(
        fs,
        if libs.is_empty() { None } else { Some(libs) },
        clock,
        cwd,
        lib_path,
        test.env.clone(),
        test.output_is_tty.unwrap_or(true),
    )))
}

fn apply_ops(sys: &TestSys, ops: &[scenarioOp]) {
    let fs = sys.fs_from_file_map();
    for op in ops {
        match op.op.as_str() {
            "write" => fs.write_file(&op.path, &op.content).unwrap(),
            "delete" => fs.remove(&op.path).unwrap(),
            "symlink" => {
                let dir = tspath::get_directory_path(&op.path);
                let _ = sys.map_fs().mkdir_all(dir.trim_start_matches('/'), tsrs_vfs::FileMode::from_bits_retain(0o777));
                sys.map_fs().add_symlink(&op.path, &op.target)
            }
            "touch" => {
                let now = sys.clock.now();
                fs.chtimes(&op.path, now, now).unwrap()
            }
            other => panic!("unknown op {other}"),
        }
    }
}

// runner.go executeCommand
fn execute_command(sys: &'static TestSys, baseline: &mut String, args: &[String]) -> ExitStatus {
    baseline.push_str(&format!("tsgo {}\n", args.join(" ")));
    let testing: &'static dyn CommandLineTesting = sys;
    let result = crate::execute::command_line_with_testing(sys, args.to_vec(), Some(testing));
    baseline.push_str(match result.status {
        ExitStatus::Success => "ExitStatus:: Success",
        ExitStatus::DiagnosticsPresent_OutputsSkipped => "ExitStatus:: DiagnosticsPresent_OutputsSkipped",
        ExitStatus::DiagnosticsPresent_OutputsGenerated => "ExitStatus:: DiagnosticsPresent_OutputsGenerated",
        ExitStatus::InvalidProject_OutputsSkipped => "ExitStatus:: InvalidProject_OutputsSkipped",
        ExitStatus::ProjectReferenceCycle_OutputsSkipped => "ExitStatus:: ProjectReferenceCycle_OutputsSkipped",
        ExitStatus::NotImplemented => "ExitStatus:: NotImplemented",
    });
    result.status
}

// runner.go getDiffForIncremental
fn get_diff_for_incremental(incremental: &TestSys, non_incremental: &TestSys) -> String {
    let mut diff = String::new();
    let mut outputs: Vec<String> = non_incremental.fs.written_files.lock().unwrap().iter().cloned().collect();
    outputs.sort();
    for output in outputs {
        if tspath::file_extension_is(&output, tspath::EXTENSION_TS_BUILD_INFO) || output.ends_with(".readable.baseline.txt") {
            // Just check existence
            if !incremental.fs_from_file_map().file_exists(&output) {
                diff.push_str(&diff_text(&format!("nonIncremental {output}"), &format!("incremental {output}"), "Exists", ""));
                diff.push('\n');
            }
        } else {
            let non_incremental_text = non_incremental.fs_from_file_map().read_file(&output).unwrap_or_else(|| panic!("Written file not found {output}"));
            let incremental_text = incremental.fs_from_file_map().read_file(&output);
            if incremental_text.as_deref() != Some(non_incremental_text.as_str()) {
                diff.push_str(&diff_text(
                    &format!("nonIncremental {output}"),
                    &format!("incremental {output}"),
                    &non_incremental_text,
                    incremental_text.as_deref().unwrap_or(""),
                ));
                diff.push('\n');
            }
        }
    }
    let incremental_output = incremental.get_output(true);
    let non_incremental_output = non_incremental.get_output(true);
    if incremental_output != non_incremental_output {
        diff.push_str(&diff_text("nonIncremental.output.txt", "incremental.output.txt", &non_incremental_output, &incremental_output));
    }
    diff
}

// baseline.go:31 DiffText (patience diff, 3 lines of context), as in tsrs_testrunner.
fn diff_text(old_name: &str, new_name: &str, expected: &str, actual: &str) -> String {
    let diff = similar::TextDiff::configure().algorithm(similar::Algorithm::Patience).diff_lines(expected, actual);
    let mut text = diff.unified_diff().context_radius(3).header(old_name, new_name).to_string();
    // Go's patience.UnifiedDiffText does not end with a newline.
    if text.ends_with('\n') {
        text.pop();
    }
    text
}

// runner.go run
fn run_scenario(test: &scenario) -> String {
    let mut baseline = String::new();
    let sys = new_test_sys(test);
    baseline.push_str(&format!(
        "currentDirectory::{}\nuseCaseSensitiveFileNames::{}\nInput::\n",
        sys.cwd,
        sys.fs.use_case_sensitive_file_names()
    ));
    sys.baseline_fs_with_diff(&mut baseline);
    execute_command(sys, &mut baseline, &test.args);
    sys.serialize_state(&mut baseline);
    sys.baseline_programs(&mut baseline, "Initial build");

    for (index, edit) in test.edits.iter().enumerate() {
        sys.clear_output();
        let args = edit.args.clone().unwrap_or_else(|| test.args.clone());
        baseline.push_str(&format!("\n\nEdit [{}]:: {}\n", index, edit.caption));
        apply_ops(sys, &edit.ops);
        sys.baseline_fs_with_diff(&mut baseline);
        execute_command(sys, &mut baseline, &args);
        sys.serialize_state(&mut baseline);
        sys.baseline_programs(&mut baseline, &format!("Edit [{}]:: {}\n", index, edit.caption));

        // Compute build with all the edits
        let non_incremental_sys = new_test_sys(test);
        apply_ops(non_incremental_sys, &edit.fresh_ops);
        let testing: &'static dyn CommandLineTesting = non_incremental_sys;
        crate::execute::command_line_with_testing(non_incremental_sys, args.clone(), Some(testing));

        let diff = get_diff_for_incremental(sys, non_incremental_sys);
        if !diff.is_empty() {
            baseline.push_str(&format!(
                "\n\nDiff:: {}\n",
                if edit.expected_diff.is_empty() {
                    "!!! Unexpected diff, please review and either fix or write explanation as expectedDiff !!!"
                } else {
                    &edit.expected_diff
                }
            ));
            baseline.push_str(&diff);
        } else if !edit.expected_diff.is_empty() {
            baseline.push_str(&format!(
                "\n\nDiff:: {} !!! Diff not found but explanation present, please review and remove the explanation !!!\n",
                edit.expected_diff
            ));
        }
    }
    baseline
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn collect(dir: &FsPath, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries {
        let p = e.unwrap().path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

#[test]
fn tsctests() {
    // The baselines are tsgo's output: keep Go's check history (tsrs_core::compat).
    tsrs_core::compat::use_go_history_for_tsgo_baselines();
    let root = repo_root();
    let dump = std::env::var("TSCTESTS_DUMP").map(PathBuf::from).unwrap_or_else(|_| root.join("target/tsctests-dump"));
    let filter = std::env::var("TSCTESTS_FILTER").unwrap_or_default();
    let mut files = Vec::new();
    collect(&dump.join("tsc"), &mut files);
    collect(&dump.join("tsbuild"), &mut files);
    files.sort();
    if files.is_empty() {
        eprintln!("tsctests: no scenarios under {} (run tools/oracle/tsctests/dump.sh)", dump.display());
        return;
    }
    let scenarios: Vec<scenario> = files
        .iter()
        .map(|f| load_scenario(f))
        .filter(|s| !s.baseline.contains("contentMapper") && s.baseline.contains(&filter))
        .collect();
    let results_dir = root.join("target/tsctests-results");
    let _ = std::fs::remove_dir_all(&results_dir);
    std::fs::create_dir_all(&results_dir).unwrap();
    let results: Mutex<Vec<(String, &'static str)>> = Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let jobs = std::env::var("TSCTESTS_JOBS").ok().and_then(|j| j.parse().ok()).unwrap_or(8);
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(test) = scenarios.get(i) else { break };
                let test_ref: &'static scenario = unsafe { &*(test as *const scenario) };
                let outcome = std::thread::Builder::new()
                    .stack_size(512 << 20)
                    .spawn(move || std::panic::catch_unwind(|| run_scenario(test_ref)))
                    .unwrap()
                    .join()
                    .unwrap();
                let reference = std::fs::read_to_string(root.join("ts-ref/tsc/testdata/baselines/reference").join(&test.baseline)).ok();
                let class = match outcome {
                    Ok(actual) => {
                        if reference.as_deref() == Some(actual.as_str()) {
                            "pass"
                        } else {
                            let local = results_dir.join("local").join(&test.baseline);
                            std::fs::create_dir_all(local.parent().unwrap()).unwrap();
                            std::fs::write(local, actual).unwrap();
                            "fail"
                        }
                    }
                    Err(payload) => {
                        let msg = payload
                            .downcast_ref::<String>()
                            .cloned()
                            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                            .unwrap_or_default();
                        let local = results_dir.join("local").join(format!("{}.crash", test.baseline));
                        std::fs::create_dir_all(local.parent().unwrap()).unwrap();
                        std::fs::write(local, msg).unwrap();
                        "crash"
                    }
                };
                results.lock().unwrap().push((test.baseline.clone(), class));
            });
        }
    });
    std::panic::set_hook(prev_hook);
    let mut results = results.into_inner().unwrap();
    results.sort();
    let mut summary = String::new();
    for class in ["pass", "fail", "crash"] {
        let names: Vec<&str> = results.iter().filter(|r| r.1 == class).map(|r| r.0.as_str()).collect();
        std::fs::write(results_dir.join(format!("{class}.txt")), names.join("\n") + "\n").unwrap();
        for suite in ["tsc/", "tsbuild/"] {
            let total = results.iter().filter(|r| r.0.starts_with(suite)).count();
            let n = names.iter().filter(|n| n.starts_with(suite)).count();
            summary += &format!("{suite:9}{class:6} {n:4} / {total}\n");
        }
    }
    eprintln!("tsctests (non-watch, no content mappers):\n{summary}results in {}", results_dir.display());
}
