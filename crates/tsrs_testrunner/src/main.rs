// The alloc-profile build installs tsrs_core's counting allocator (over mimalloc) instead.
#[cfg(not(feature = "alloc-profile"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod baseline;
#[cfg(feature = "compiler")]
mod compile;
mod compiler_runner;
mod diagnosticwriter;
mod harnessutil;
#[cfg(feature = "tsoptions")]
mod options;
mod oracle;
mod pool;
mod report;
mod test_case_parser;
mod tsbaseline;
#[cfg(feature = "checker")]
mod type_symbol_baseline;
#[cfg(feature = "checker")]
mod types_dump;
mod worker;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use regex::Regex;
use rustc_hash::FxHashSet;
use tsrs_core::tspath;

use crate::baseline::Class;
use crate::compiler_runner::{Outcome, TestItem, SUITES};
use crate::harnessutil::OptionTable;

const USAGE: &str = "usage:
  tsrs-test run [--suite compiler|conformance|all] [--filter <substr|regex>] [--list <file>]
                [--jobs N] [--timeout S] [--recycle N] [--mem-limit MB] [--json <path>] [--panic-summary]
                [--baselines types,symbols | --types --symbols]   also compare .types/.symbols baselines
                  (results: <suite>/<name>.{types,symbols}.{actual,diff}, lists types-<class>.txt, symbols-<class>.txt)
  tsrs-test show <name> [--full] [--types] [--symbols]
                                      expected vs actual for one test (id, variant stem or file name); with
                                      --types/--symbols: the first differing hunk of those baselines (--full: whole diff)
  tsrs-test crashes [--top N] [--examples N] [--json <path>]
  tsrs-test list [--suite ..] [--filter ..] [--list <file>]
  tsrs-test types-dump -p <tsconfig|dir> --out <dir> [--mode types|symbols|both] [--text all|none|<list file>]
                      [--sample <list file>]
                                      the .types/.symbols walk over every non-node_modules, non-lib file of a project
                                      (single-threaded, like tools/oracle/project-types): <out>/manifest.<kind>
                                      (hash, lines, path per file) and <out>/<kind>/<path>.<kind>; --sample walks only
                                      the listed files (e.g. tools/project-types-sample.txt), in program order
  --syntax-only (any command): no checker; only config/program/syntactic diagnostics; results in target/test-results-syntax
                (implied when built without the `checker` feature)
dev options (any command): --oracle <diags.jsonl> render Go-captured diagnostics instead of compiling;
                           --options <options.json> option table dumped by tools/oracle/testrunner";

// Early-phase mode: collect only config-file, program and syntactic diagnostics (no checker), with results
// under target/test-results-syntax.
pub static SYNTAX_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn syntax_only() -> bool {
    SYNTAX_ONLY.load(std::sync::atomic::Ordering::Relaxed)
}

// Which extra baselines to generate and compare besides `.errors.txt` (bit set of EXTRA_TYPES / EXTRA_SYMBOLS).
// Asking for either runs both walks (Go runs the type walk before the symbol walk in the same program).
pub static EXTRA_BASELINES: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
pub const EXTRA_TYPES: u8 = 1;
pub const EXTRA_SYMBOLS: u8 = 2;

pub fn extra_baselines() -> u8 {
    EXTRA_BASELINES.load(std::sync::atomic::Ordering::Relaxed)
}

fn parse_baselines(v: &str) -> u8 {
    let mut bits = 0;
    for part in v.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        bits |= match part {
            "errors" => 0,
            "types" => EXTRA_TYPES,
            "symbols" => EXTRA_SYMBOLS,
            "all" => EXTRA_TYPES | EXTRA_SYMBOLS,
            _ => {
                eprintln!("--baselines: unknown baseline kind {part} (errors, types, symbols, all)");
                std::process::exit(2)
            }
        };
    }
    bits
}

#[derive(Clone, Default)]
pub struct BackendSpec {
    pub oracle: Option<String>,
    pub options: Option<String>,
}

impl BackendSpec {
    fn args(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(o) = &self.oracle {
            v.extend(["--oracle".to_string(), o.clone()]);
        }
        if let Some(o) = &self.options {
            v.extend(["--options".to_string(), o.clone()]);
        }
        if syntax_only() {
            v.push("--syntax-only".to_string());
        }
        if extra_baselines() != 0 {
            let mut kinds = Vec::new();
            if extra_baselines() & EXTRA_TYPES != 0 {
                kinds.push("types");
            }
            if extra_baselines() & EXTRA_SYMBOLS != 0 {
                kinds.push("symbols");
            }
            v.extend(["--baselines".to_string(), kinds.join(",")]);
        }
        v
    }
}

pub struct Backend {
    oracle: Option<oracle::Oracle>,
    table: OptionTable,
}

pub fn option_table(spec: &BackendSpec) -> OptionTable {
    let path = spec.options.clone().or_else(|| std::env::var("TSRS_TEST_OPTIONS").ok());
    #[cfg(feature = "tsoptions")]
    if path.is_none() {
        return options::tsoptions_option_table();
    }
    let path = path
        .unwrap_or_else(|| compiler_runner::repo_root().join("target/scratch/testrunner/options.json").to_string_lossy().into_owned());
    oracle::load_option_table(&path).unwrap_or_else(|e| panic!("option table: {e}"))
}

impl Backend {
    pub fn new(spec: &BackendSpec) -> Backend {
        Backend { oracle: spec.oracle.as_deref().map(oracle::Oracle::load), table: option_table(spec) }
    }

    pub fn run(&self, item: &TestItem) -> Outcome {
        if let Some(o) = &self.oracle {
            return o.run(item);
        }
        #[cfg(feature = "compiler")]
        return compile::run(item, &self.table);
        #[cfg(not(feature = "compiler"))]
        Outcome::Error("built without the `compiler` feature".to_string())
    }
}

struct Args {
    rest: Vec<String>,
}

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        if let Some(i) = self.rest.iter().position(|a| a == name) {
            self.rest.remove(i);
            return true;
        }
        false
    }
    fn value(&mut self, name: &str) -> Option<String> {
        let i = self.rest.iter().position(|a| a == name)?;
        if i + 1 >= self.rest.len() {
            eprintln!("missing value for {name}");
            std::process::exit(2);
        }
        let v = self.rest.remove(i + 1);
        self.rest.remove(i);
        Some(v)
    }
    fn num(&mut self, name: &str) -> Option<usize> {
        self.value(name).map(|v| {
            v.parse().unwrap_or_else(|_| {
                eprintln!("{name}: expected a number");
                std::process::exit(2)
            })
        })
    }
}

struct Selection {
    suites: Vec<&'static str>,
    filter: Option<String>,
    list: Option<FxHashSet<String>>,
}

impl Selection {
    fn from_args(args: &mut Args) -> Selection {
        let suites = match args.value("--suite").as_deref() {
            None | Some("all") => SUITES.to_vec(),
            Some("compiler") => vec!["compiler"],
            Some("conformance") => vec!["conformance"],
            Some(s) => {
                eprintln!("unknown suite {s}");
                std::process::exit(2)
            }
        };
        let filter = args.value("--filter");
        let list = args.value("--list").map(|p| {
            std::fs::read_to_string(&p)
                .unwrap_or_else(|e| panic!("--list {p}: {e}"))
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect()
        });
        Selection { suites, filter, list }
    }

    fn is_partial(&self) -> bool {
        self.suites.len() != SUITES.len() || self.filter.is_some() || self.list.is_some()
    }

    fn select(&self, table: &OptionTable) -> Vec<TestItem> {
        let re = self.filter.as_deref().and_then(|f| Regex::new(f).ok());
        let mut items = Vec::new();
        for suite in &self.suites {
            for path in compiler_runner::enumerate_test_files(suite) {
                let basename = tspath::get_base_file_name(&path);
                let expanded = match compiler_runner::expand_test_file(suite, &path, table) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("{suite}/{basename}: {e}");
                        vec![TestItem {
                            suite: suite.to_string(),
                            path: path.clone(),
                            config: String::new(),
                            name: compiler_runner::baseline_stem(&basename),
                        }]
                    }
                };
                for item in expanded {
                    let id = item.id();
                    if let Some(f) = &self.filter {
                        if !id.contains(f.as_str()) && !re.as_ref().is_some_and(|r| r.is_match(&id)) {
                            continue;
                        }
                    }
                    if let Some(list) = &self.list {
                        if !list.contains(&id) && !list.contains(&item.name) && !list.contains(&basename) {
                            continue;
                        }
                    }
                    items.push(item);
                }
            }
        }
        items
    }
}

fn cmd_run(mut args: Args, spec: BackendSpec) {
    let sel = Selection::from_args(&mut args);
    let jobs = args.num("--jobs").unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    let timeout = Duration::from_secs_f64(args.value("--timeout").map_or(20.0, |v| v.parse().unwrap_or(20.0)));
    let recycle = args.num("--recycle").unwrap_or(200);
    let mem_limit_mb = args.num("--mem-limit").unwrap_or(6144) as u64;
    let json_path = args.value("--json").map(PathBuf::from);
    let panic_summary = args.flag("--panic-summary");
    let quiet = args.flag("--quiet");
    check_no_extra(&args);

    let started = Instant::now();
    let table = option_table(&spec);
    let items = sel.select(&table);
    if items.is_empty() {
        println!("no tests selected");
        return;
    }
    if !quiet {
        let mode = if syntax_only() { " (syntax-only)" } else { "" };
        eprintln!("running {} test variants with {jobs} workers{mode}", items.len());
    }
    let opts = pool::PoolOptions { jobs: jobs.min(items.len()), timeout, recycle, worker_args: spec.args(), progress: !quiet, mem_limit_mb };
    let results = pool::run_pool(&items, &opts);

    let dir = worker::results_dir();
    let json_path = json_path.unwrap_or_else(|| dir.join("summary.json"));
    let previous = report::load_summary(&json_path);
    let mut summary = if sel.is_partial() { previous.clone() } else { report::Summary::new() };
    let mut this_run = report::Summary::new();
    for (item, r) in items.iter().zip(&results) {
        let mut r = r.clone().unwrap_or(pool::TestResult::harness(Class::Crash, 0, "no result from worker".to_string()));
        // A worker that died or timed out produced no `.types`/`.symbols` result: they share the item's fate.
        if matches!(r.class, Class::Crash | Class::Timeout) {
            if extra_baselines() & EXTRA_TYPES != 0 && r.types.is_none() {
                r.types = Some((r.class, r.panic.clone()));
            }
            if extra_baselines() & EXTRA_SYMBOLS != 0 && r.symbols.is_none() {
                r.symbols = Some((r.class, r.panic.clone()));
            }
        }
        let mut entry = report::Entry::from(&r);
        // An errors-only (or types-only) run keeps the other baselines' previous results.
        if let Some(old) = previous.get(&item.id()) {
            if entry.types.is_none() {
                entry.types = old.types.clone();
            }
            if entry.symbols.is_none() {
                entry.symbols = old.symbols.clone();
            }
        }
        summary.insert(item.id(), entry);
        this_run.insert(item.id(), report::Entry::from(&r));
    }
    report::write_summary(&dir, &json_path, &summary);
    report::print_table(&this_run);
    println!("{:.1}s; results in {}", started.elapsed().as_secs_f64(), dir.display());
    if syntax_only() {
        // Tests whose reference baseline has no diagnostics or only TS1xxx ones are the ones a
        // parser-only compiler can be expected to pass.
        let (mut eligible, mut passing) = (0, 0);
        for (item, r) in items.iter().zip(&results) {
            let expected = compiler_runner::read_reference_baseline(&item.suite, &item.name);
            let keys = baseline::diagnostic_keys(expected.as_deref().unwrap_or(baseline::NO_CONTENT));
            if keys.iter().all(|k| k.rsplit(" TS").next().is_some_and(|c| c.len() == 4 && c.starts_with('1'))) {
                eligible += 1;
                if r.as_ref().is_some_and(|r| r.class == Class::Pass) {
                    passing += 1;
                }
            }
        }
        println!("syntax-only: {passing}/{eligible} pass among tests whose reference baseline has only TS1xxx (or no) errors");
    }
    if panic_summary {
        report::print_crashes(&this_run, 15, 2);
    }
}

fn check_no_extra(args: &Args) {
    if !args.rest.is_empty() {
        eprintln!("unexpected arguments: {}\n{USAGE}", args.rest.join(" "));
        std::process::exit(2);
    }
}

fn cmd_list(mut args: Args, spec: BackendSpec) {
    let sel = Selection::from_args(&mut args);
    check_no_extra(&args);
    for item in sel.select(&option_table(&spec)) {
        println!("{}", item.id());
    }
}

fn cmd_show(mut args: Args, spec: BackendSpec) {
    let full = args.flag("--full");
    if args.rest.len() != 1 {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let name = args.rest.remove(0);
    let table = option_table(&spec);
    let sel = Selection { suites: SUITES.to_vec(), filter: None, list: None };
    let all = sel.select(&table);
    let exact: Vec<&TestItem> = all
        .iter()
        .filter(|i| i.id() == name || i.name == name || tspath::get_base_file_name(&i.path) == name || compiler_runner::baseline_stem(&name) == i.name)
        .collect();
    let matches: Vec<&TestItem> = if exact.is_empty() { all.iter().filter(|i| i.id().contains(&name)).collect() } else { exact };
    if matches.is_empty() {
        println!("no test matches '{name}'");
        return;
    }
    if matches.len() > 1 && matches.iter().any(|m| m.path != matches[0].path) {
        println!("'{name}' is ambiguous:");
        for m in matches.iter().take(20) {
            println!("  {}", m.id());
        }
        return;
    }
    worker::install_panic_hook(true);
    let items: Vec<TestItem> = matches.into_iter().cloned().collect();
    std::thread::Builder::new()
        .stack_size(worker::WORKER_STACK_SIZE)
        .spawn(move || {
            let backend = Backend::new(&spec);
            for item in &items {
                let r = worker::run_item(&backend, item);
                println!("== {}: {} ({} ms)", item.id(), r.class.as_str(), r.ms);
                if let Some(p) = &r.panic {
                    println!("panic: {}\n  at {}", p.message, p.location);
                }
                if !r.skip.is_empty() {
                    println!("skipped: {}", r.skip);
                }
                if r.class != Class::Pass && r.actual.is_none() && !r.diff.is_empty() {
                    println!("{}", r.diff);
                }
                for (ext, _) in worker::EXTRA_KINDS {
                    let Some(e) = worker::extra_result(&r, ext) else { continue };
                    println!("-- .{ext}: {}", e.class.as_str());
                    if let Some(actual) = &e.actual {
                        if e.class != Class::Pass {
                            let diff = baseline::unified_diff(e.expected.as_deref(), actual, &format!("{}.{ext}", item.name));
                            print!("{}", if full { diff } else { baseline::first_hunk(&diff) });
                        }
                    } else if !e.diff.is_empty() {
                        println!("{}", e.diff);
                    }
                }
                if extra_baselines() != 0 && !full {
                    continue;
                }
                if let Some(actual) = &r.actual {
                    if full {
                        println!("-- expected:\n{}", r.expected.as_deref().unwrap_or(baseline::NO_CONTENT).replace("\r\n", "\n"));
                        println!("-- actual:\n{}", actual.replace("\r\n", "\n"));
                    } else if r.class != Class::Pass {
                        print!("{}", baseline::unified_diff(r.expected.as_deref(), actual, &format!("{}.errors.txt", item.name)));
                    }
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

fn cmd_crashes(mut args: Args) {
    let top = args.num("--top").unwrap_or(30);
    let examples = args.num("--examples").unwrap_or(2);
    let json_path = args.value("--json").map(PathBuf::from).unwrap_or_else(|| worker::results_dir().join("summary.json"));
    check_no_extra(&args);
    let summary = report::load_summary(&json_path);
    if summary.is_empty() {
        println!("no results in {}", json_path.display());
        return;
    }
    report::print_crashes(&summary, top, examples);
}

#[cfg(feature = "checker")]
fn cmd_types_dump(mut args: Args) {
    let project = args.value("-p").or_else(|| args.value("--project")).unwrap_or_else(|| ".".to_string());
    let out = PathBuf::from(args.value("--out").unwrap_or_else(|| {
        eprintln!("types-dump: --out <dir> is required");
        std::process::exit(2)
    }));
    let mode = args.value("--mode").unwrap_or_else(|| "types".to_string());
    let text = args.value("--text").unwrap_or_else(|| "all".to_string());
    let sample = args.value("--sample");
    check_no_extra(&args);
    let dump = types_dump::DumpArgs { project, out, mode, text, sample };
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || types_dump::run(dump))
        .unwrap()
        .join()
        .unwrap_or_else(|_| std::process::exit(101));
}

fn main() {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let cmd = argv.remove(0);
    let mut args = Args { rest: argv };
    let spec = BackendSpec { oracle: args.value("--oracle"), options: args.value("--options") };
    if args.flag("--syntax-only") || cfg!(not(feature = "checker")) {
        SYNTAX_ONLY.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let mut extra = args.value("--baselines").map_or(0, |v| parse_baselines(&v));
    if args.flag("--types") {
        extra |= EXTRA_TYPES;
    }
    if args.flag("--symbols") {
        extra |= EXTRA_SYMBOLS;
    }
    EXTRA_BASELINES.store(extra, std::sync::atomic::Ordering::Relaxed);
    match cmd.as_str() {
        "run" => cmd_run(args, spec),
        "show" => cmd_show(args, spec),
        "crashes" => cmd_crashes(args),
        "list" => cmd_list(args, spec),
        #[cfg(feature = "checker")]
        "types-dump" => cmd_types_dump(args),
        "__worker" => worker::worker_main(spec),
        "-h" | "--help" | "help" => println!("{USAGE}"),
        _ => {
            eprintln!("unknown command {cmd}\n{USAGE}");
            std::process::exit(2);
        }
    }
}