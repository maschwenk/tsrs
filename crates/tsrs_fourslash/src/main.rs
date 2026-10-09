// tsrs-fourslash: runs the fourslash tests generated from the Go test files (tools/gen-fourslash).
//
//   tsrs-fourslash run [--filter <substr|regex>] [--include-skipped] [-j N] [-v]
//   tsrs-fourslash list [--filter <substr|regex>]

use std::process::ExitCode;

use tsrs_fourslash::runner::{self, RunOptions};
use tsrs_fourslash::tests::r#gen::REGISTRY;

fn usage() -> ExitCode {
    eprintln!("usage: tsrs-fourslash run [--filter <substr|regex>] [--include-skipped] [-j N] [-v]");
    eprintln!("       tsrs-fourslash list [--filter <substr|regex>]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    // The baselines are tsgo's output: keep Go's check history (tsrs_core::compat).
    tsrs_core::compat::use_go_history_for_tsgo_baselines();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        return usage();
    };
    let mut filter = None;
    let mut include_skipped = false;
    let mut verbose = false;
    let mut jobs = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--filter" if i + 1 < args.len() => {
                i += 1;
                match regex::Regex::new(&args[i]) {
                    Ok(re) => filter = Some(re),
                    Err(_) => filter = Some(regex::Regex::new(&regex::escape(&args[i])).unwrap()),
                }
            }
            "--include-skipped" => include_skipped = true,
            "-v" | "--verbose" => verbose = true,
            "-j" if i + 1 < args.len() => {
                i += 1;
                jobs = args[i].parse().unwrap_or(jobs);
            }
            _ => return usage(),
        }
        i += 1;
    }
    match cmd.as_str() {
        "run" => {
            let failures = runner::run(REGISTRY, &RunOptions { filter, include_skipped, jobs, verbose });
            if failures > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        "worker" => {
            runner::worker(REGISTRY, include_skipped);
            ExitCode::SUCCESS
        }
        "list" => {
            for e in REGISTRY.iter().filter(|e| filter.as_ref().is_none_or(|re| re.is_match(e.name))) {
                match e.skip {
                    Some(reason) => println!("{}\t{}:{}\tskip: {}", e.name, e.file, e.line, reason),
                    None => println!("{}\t{}:{}", e.name, e.file, e.line),
                }
            }
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}
