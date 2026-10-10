// TSRS_ASSIGNMENT_STATS=1 report: how the checker pool's file assignment shows up in each checker's work.
// Read-only over the checkers after checking (no checker code runs), printed after `--extendedDiagnostics`.
//
// "Touched" is measured from the checkers' own caches: a file is touched by a checker when the checker holds
// node links for one of its nodes or value/declared-type links for one of its declaration symbols (or for the
// checker's merged copy of that symbol). Linked nodes + linked symbols per file are the work proxy.

use std::fmt::Write;
use std::sync::Mutex;

use tsrs_ast::{Node, SourceFile};
use tsrs_core::P;

use crate::checkerpool::Checker;
use crate::program::Program;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Category {
    Lib,
    NodeModules,
    ProjectSource,
    ProjectDeclaration,
    OtherSource,
    OtherDeclaration,
}

const CATEGORIES: [(Category, &str); 6] = [
    (Category::Lib, "lib"),
    (Category::NodeModules, "node_modules"),
    (Category::ProjectSource, "project .ts"),
    (Category::ProjectDeclaration, "project .d.ts"),
    (Category::OtherSource, "workspace .ts"),
    (Category::OtherDeclaration, "workspace .d.ts"),
];

fn category(program: &Program, file: P<SourceFile>, project_dir: &str) -> Category {
    let name = file.file_name();
    if program.is_source_file_default_library(&file.path()) {
        Category::Lib
    } else if name.contains("/node_modules/") {
        Category::NodeModules
    } else if name.starts_with(project_dir) {
        if file.is_declaration_file.get() { Category::ProjectDeclaration } else { Category::ProjectSource }
    } else if file.is_declaration_file.get() {
        Category::OtherDeclaration
    } else {
        Category::OtherSource
    }
}

fn has_symbol_links(c: &Checker, symbol: P<tsrs_ast::Symbol>) -> bool {
    // value_symbol_links.has assigns a symbol id to symbols that have none; harmless after checking.
    let linked = |s: P<tsrs_ast::Symbol>| c.value_symbol_links.has(s) || c.declared_type_links.has(s);
    linked(symbol) || c.merged_symbols.get(&symbol).is_some_and(|&m| linked(m))
}

// (linked nodes, linked declaration symbols) of one file in one checker.
fn file_work(c: &Checker, file: P<SourceFile>) -> (u64, u64) {
    let mut nodes = 0u64;
    let mut symbols = 0u64;
    let mut stack: Vec<P<Node>> = vec![file.as_node()];
    while let Some(node) = stack.pop() {
        if c.node_links.has(node) {
            nodes += 1;
        }
        if let Some(symbol) = node.symbol() {
            if has_symbol_links(c, symbol) {
                symbols += 1;
            }
        }
        node.for_each_child(&mut |child| {
            stack.push(child);
            false
        });
    }
    (nodes, symbols)
}

pub(crate) fn report(program: &Program, pool: &crate::checkerpool::checkerPool) -> String {
    let state = pool.state();
    let files = &program.files;
    let k = pool.checker_count();
    let project_dir = format!("{}/", program.get_current_directory().trim_end_matches('/'));
    if std::env::var("TSRS_ASSIGNMENT_STATS").is_ok_and(|v| v == "times") {
        let mut out = String::new();
        for run in state.group_runs.lock().unwrap().iter() {
            let times: Vec<String> = run.iter().map(|(t, n)| format!("{:.2}s/{}", t, n)).collect();
            let _ = writeln!(out, "checker group seconds/files: {}", times.join(" "));
        }
        for run in state.group_cpu.lock().unwrap().iter() {
            let times: Vec<String> = run.iter().map(|t| format!("{t:.2}")).collect();
            let _ = writeln!(out, "checker group cpu seconds: {}", times.join(" "));
        }
        for run in state.group_stolen.lock().unwrap().iter() {
            let counts: Vec<String> = run.iter().map(|n| n.to_string()).collect();
            let _ = writeln!(out, "checker group stolen files: {}", counts.join(" "));
        }
        return out;
    }
    let categories: Vec<Category> = files.iter().map(|&f| category(program, f, &project_dir)).collect();

    // work[c][file] = linked nodes + linked symbols
    let work: Vec<Mutex<Vec<u64>>> = (0..k).map(|_| Mutex::new(Vec::new())).collect();
    let counters: Vec<Mutex<(u64, u64, u64)>> = (0..k).map(|_| Mutex::new((0, 0, 0))).collect();
    pool.for_each_checker_parallel(|idx, c| {
        let w: Vec<u64> = files
            .iter()
            .map(|&f| {
                let (n, s) = file_work(c, f);
                n + s
            })
            .collect();
        *work[idx].lock().unwrap() = w;
        *counters[idx].lock().unwrap() = (c.symbol_count as u64, c.type_count as u64, c.total_instantiation_count as u64);
    });
    #[cfg(feature = "assignment-stats")]
    let created = created_by_file(program, pool);
    let work: Vec<Vec<u64>> = work.into_iter().map(|w| w.into_inner().unwrap()).collect();
    let counters: Vec<(u64, u64, u64)> = counters.into_iter().map(|c| c.into_inner().unwrap()).collect();

    let mut out = String::new();
    let _ = writeln!(out, "\n== checker assignment stats ({} checkers, {} files, project dir {}) ==", k, files.len(), project_dir);
    let runs = state.group_runs.lock().unwrap();
    for (run_index, run) in runs.iter().enumerate() {
        let times: Vec<String> = run.iter().map(|(t, n)| format!("{:.2}s/{}", t, n)).collect();
        let _ = writeln!(out, "group run {}: seconds/files per checker: {}", run_index, times.join(" "));
    }

    let _ = writeln!(
        out,
        "{:>3} {:>8} {:>8} {:>11} {:>11} {:>12} {:>9} {:>9} {:>7} {:>7} {:>7} {:>7}",
        "chk", "own src", "own decl", "symbols", "types", "inst", "touched", "work(M)", "own%", "fsrc%", "decl%", "lib%"
    );
    for c in 0..k {
        let mut own_src = 0;
        let mut own_decl = 0;
        let mut touched = 0;
        let (mut total, mut own, mut foreign_src, mut decl, mut lib) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for (i, &f) in files.iter().enumerate() {
            let mine = state.owner_at(i) == c;
            if mine {
                if f.is_declaration_file.get() { own_decl += 1 } else { own_src += 1 }
            }
            let w = work[c][i];
            if w == 0 {
                continue;
            }
            touched += 1;
            total += w;
            match categories[i] {
                Category::Lib => lib += w,
                _ if f.is_declaration_file.get() => decl += w,
                _ if mine => own += w,
                _ => foreign_src += w,
            }
        }
        let pct = |x: u64| if total == 0 { 0.0 } else { 100.0 * x as f64 / total as f64 };
        let _ = writeln!(
            out,
            "{:>3} {:>8} {:>8} {:>11} {:>11} {:>12} {:>9} {:>9.2} {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}%",
            c,
            own_src,
            own_decl,
            counters[c].0,
            counters[c].1,
            counters[c].2,
            touched,
            total as f64 / 1e6,
            pct(own),
            pct(foreign_src),
            pct(decl),
            pct(lib)
        );
    }

    // Sharing: per category, files and work by the number of checkers that touched them.
    let _ = writeln!(out, "sharing by category (files touched by exactly n checkers; work summed over checkers, M):");
    let mut header = format!("{:>16} {:>7}", "category", "files");
    for n in 0..=k {
        let _ = write!(header, " {:>14}", format!("n={n}"));
    }
    let _ = write!(header, " {:>9} {:>9}", "work", "dup");
    let _ = writeln!(out, "{header}");
    let mut grand_sum = 0u64;
    let mut grand_max = 0u64;
    for &(cat, name) in &CATEGORIES {
        let mut by_n_files = vec![0usize; k + 1];
        let mut by_n_work = vec![0u64; k + 1];
        let mut sum = 0u64;
        let mut max = 0u64;
        let mut count = 0;
        for i in 0..files.len() {
            if categories[i] != cat {
                continue;
            }
            count += 1;
            let n = (0..k).filter(|&c| work[c][i] > 0).count();
            let s: u64 = (0..k).map(|c| work[c][i]).sum();
            by_n_files[n] += 1;
            by_n_work[n] += s;
            sum += s;
            max += (0..k).map(|c| work[c][i]).max().unwrap_or(0);
        }
        grand_sum += sum;
        grand_max += max;
        let mut line = format!("{:>16} {:>7}", name, count);
        for n in 0..=k {
            let _ = write!(line, " {:>14}", format!("{}/{:.2}", by_n_files[n], by_n_work[n] as f64 / 1e6));
        }
        let _ = write!(line, " {:>9.2} {:>8.2}x", sum as f64 / 1e6, if max == 0 { 0.0 } else { sum as f64 / max as f64 });
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(
        out,
        "total work {:.2}M, max-per-file (~single-checker) {:.2}M, duplication {:.2}x",
        grand_sum as f64 / 1e6,
        grand_max as f64 / 1e6,
        if grand_max == 0 { 0.0 } else { grand_sum as f64 / grand_max as f64 }
    );
    #[cfg(feature = "assignment-stats")]
    report_created(&mut out, program, state, &categories, &created);
    out
}

// created[c] = (types per file index, symbols per file index, types without a file, symbols without a file):
// a checker-created type is attributed to the first declaration of its symbol (or alias symbol), a
// checker-created symbol to its first declaration.
#[cfg(feature = "assignment-stats")]
type Created = (Vec<u64>, Vec<u64>, u64, u64);

#[cfg(feature = "assignment-stats")]
fn created_by_file(program: &Program, pool: &crate::checkerpool::checkerPool) -> Vec<Created> {
    let files = &program.files;
    let index: rustc_hash::FxHashMap<P<SourceFile>, usize> = files.iter().enumerate().map(|(i, &f)| (f, i)).collect();
    let file_of = |symbol: Option<P<tsrs_ast::Symbol>>| -> Option<usize> {
        let decl = *symbol?.declarations().first()?;
        index.get(&tsrs_ast::get_source_file_of_node(decl)?).copied()
    };
    let k = pool.checker_count();
    let result: Vec<Mutex<Created>> = (0..k).map(|_| Mutex::new((Vec::new(), Vec::new(), 0, 0))).collect();
    pool.for_each_checker_parallel(|idx, c| {
        let mut types = vec![0u64; files.len()];
        let mut symbols = vec![0u64; files.len()];
        let (mut no_type_file, mut no_symbol_file) = (0, 0);
        for &t in &c.stats_created.0 {
            let symbol = t.symbol().or_else(|| t.alias().and_then(|a| c.type_alias(a).symbol()));
            match file_of(symbol) {
                Some(i) => types[i] += 1,
                None => no_type_file += 1,
            }
        }
        for &s in &c.stats_created.1 {
            match file_of(Some(s)) {
                Some(i) => symbols[i] += 1,
                None => no_symbol_file += 1,
            }
        }
        *result[idx].lock().unwrap() = (types, symbols, no_type_file, no_symbol_file);
    });
    result.into_iter().map(|r| r.into_inner().unwrap()).collect()
}

#[cfg(feature = "assignment-stats")]
fn package_of(name: &str, project_dir: &str) -> String {
    if let Some(pos) = name.rfind("/node_modules/") {
        let rest = &name[pos + "/node_modules/".len()..];
        let mut parts = rest.split('/');
        let first = parts.next().unwrap_or("");
        return if first.starts_with('@') { format!("{}/{}", first, parts.next().unwrap_or("")) } else { first.to_string() };
    }
    if let Some(rest) = name.strip_prefix(project_dir) {
        let mut parts = rest.split('/');
        let first = parts.next().unwrap_or("");
        let second = parts.next().unwrap_or("");
        return format!("project:{first}/{second}");
    }
    if let Some(pos) = name.find("/packages/") {
        return format!("workspace:{}", name[pos + "/packages/".len()..].split('/').next().unwrap_or(""));
    }
    "other".to_string()
}

#[cfg(feature = "assignment-stats")]
fn report_created(out: &mut String, program: &Program, state: &crate::checkerpool::poolState, categories: &[Category], created: &[Created]) {
    let files = &program.files;
    let k = created.len();
    let project_dir = format!("{}/", program.get_current_directory().trim_end_matches('/'));
    for (label, which) in [("types", 0usize), ("symbols", 1usize)] {
        let get = |c: usize, i: usize| if which == 0 { created[c].0[i] } else { created[c].1[i] };
        let unattributed: Vec<u64> = (0..k).map(|c| if which == 0 { created[c].2 } else { created[c].3 }).collect();
        let _ = writeln!(out, "checker-created {label} by declaring file (M): per checker own-src / foreign-src / per category; unattributed");
        for c in 0..k {
            let mut by_cat = vec![0u64; CATEGORIES.len()];
            let (mut own, mut foreign) = (0u64, 0u64);
            for i in 0..files.len() {
                let v = get(c, i);
                if v == 0 {
                    continue;
                }
                let ci = CATEGORIES.iter().position(|&(cat, _)| cat == categories[i]).unwrap();
                by_cat[ci] += v;
                if categories[i] == Category::ProjectSource || categories[i] == Category::OtherSource {
                    if state.owner_at(i) == c { own += v } else { foreign += v }
                }
            }
            let cats: Vec<String> = CATEGORIES.iter().zip(&by_cat).map(|((_, n), v)| format!("{n}={:.2}", *v as f64 / 1e6)).collect();
            let _ = writeln!(
                out,
                "  chk {c}: own-src {:.2} foreign-src {:.2} | {} | unattributed {:.2}",
                own as f64 / 1e6,
                foreign as f64 / 1e6,
                cats.join(" "),
                unattributed[c] as f64 / 1e6
            );
        }
        // Per package: sum over checkers vs max over checkers per file (a lower bound of what one checker needs).
        let mut packages: std::collections::BTreeMap<String, (u64, u64)> = std::collections::BTreeMap::new();
        for (i, &f) in files.iter().enumerate() {
            let sum: u64 = (0..k).map(|c| get(c, i)).sum();
            if sum == 0 {
                continue;
            }
            let max = (0..k).map(|c| get(c, i)).max().unwrap_or(0);
            let e = packages.entry(package_of(f.file_name(), &project_dir)).or_default();
            e.0 += sum;
            e.1 += max;
        }
        let mut packages: Vec<(String, (u64, u64))> = packages.into_iter().collect();
        packages.sort_by(|a, b| (b.1 .0 - b.1 .1).cmp(&(a.1 .0 - a.1 .1)).then(a.0.cmp(&b.0)));
        let total_sum: u64 = packages.iter().map(|p| p.1 .0).sum();
        let total_max: u64 = packages.iter().map(|p| p.1 .1).sum();
        let _ = writeln!(
            out,
            "  {label}: attributed sum {:.2}M, per-file max {:.2}M (excess {:.2}M); top packages by excess (sum/max, M):",
            total_sum as f64 / 1e6,
            total_max as f64 / 1e6,
            (total_sum - total_max) as f64 / 1e6
        );
        for (name, (sum, max)) in packages.iter().take(25) {
            let _ = writeln!(out, "    {:<48} {:>8.3} {:>8.3} excess {:>7.3}", name, *sum as f64 / 1e6, *max as f64 / 1e6, (sum - max) as f64 / 1e6);
        }
    }
}
