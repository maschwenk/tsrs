// Throwaway probe report for the persisted front-end design (design/persisted-frontend-probe, not for main).
// `TSRS_PFE_PROBE=1 TSRS_PFE_PROBE_OUT=<tsv>`: per-file parse/bind CPU, sizes, counts and page touches.

use crate::program::Program;
use rustc_hash::{FxHashMap, FxHashSet};
use std::fmt::Write;
use tsrs_ast::SourceFile;
use tsrs_core::pfe_probe;
use tsrs_core::P;

pub(crate) fn class_of(name: &str) -> &'static str {
    if name.starts_with("bundled:///libs/") {
        "lib"
    } else if name.contains("/node_modules/") {
        if tsrs_core::tspath::is_declaration_file_name(name) { "nm-dts" } else { "nm-other" }
    } else if tsrs_core::tspath::is_declaration_file_name(name) {
        "ws-dts"
    } else {
        "project"
    }
}

#[derive(Default, Clone, Copy)]
struct Agg {
    files: usize,
    text: usize,
    lines: usize,
    nodes: usize,
    idents: usize,
    symbols: usize,
    parse_ns: u64,
    bind_ns: u64,
    used: usize,
    pages: usize,
    touched: usize,
    written: usize,
    files_touched: usize,
    grown: usize,
}

pub(crate) fn write(program: &'static Program) {
    if !pfe_probe::enabled() {
        return;
    }
    let faults = pfe_probe::faults();
    let recs = pfe_probe::files();
    // Page states first: reading the files below touches their pages.
    let stats: Vec<pfe_probe::PageStats> = recs.iter().map(|r| pfe_probe::page_stats(r)).collect();
    let mapped: Vec<usize> = recs.iter().map(|r| r.chunks.lock().unwrap().iter().map(|&(_, l)| l).sum()).collect();
    let ends: Vec<usize> = recs.iter().map(|r| r.region.allocated_bytes()).collect();
    let in_program: FxHashSet<usize> = program.files.iter().map(|f| f.addr()).collect();
    let mut out = String::from(
        "class\tin_program\ttext\tlines\tnodes\tidents\tsymbols\tparse_ns\tbind_ns\tused_bytes\tpages\ttouched\twritten\tmapped\tbytes_end\tfile\n",
    );
    let mut aggs: FxHashMap<&'static str, Agg> = FxHashMap::default();
    for (i, rec) in recs.iter().enumerate() {
        // SAFETY: registered from a live, never-freed source file.
        let file: P<SourceFile> = P::from_static(unsafe { &*(rec.file_addr as *const SourceFile) });
        let name = file.file_name();
        let class = class_of(name);
        let inp = in_program.contains(&rec.file_addr);
        let text = file.text();
        let lines = text.bytes().filter(|&b| b == b'\n').count() + 1;
        let s = &stats[i];
        let used = rec.used_at_protect.load(std::sync::atomic::Ordering::Relaxed);
        let bind_ns = rec.bind_ns.load(std::sync::atomic::Ordering::Relaxed);
        let _ = writeln!(
            out,
            "{class}\t{}\t{}\t{lines}\t{}\t{}\t{}\t{}\t{bind_ns}\t{used}\t{}\t{}\t{}\t{}\t{}\t{name}",
            inp as u8,
            text.len(),
            file.node_count.get(),
            file.identifier_count.get(),
            file.symbol_count.get(),
            rec.parse_ns,
            s.pages,
            s.touched,
            s.written,
            mapped[i],
            ends[i],
        );
        if !inp {
            continue;
        }
        let a = aggs.entry(class).or_default();
        a.files += 1;
        a.text += text.len();
        a.lines += lines;
        a.nodes += file.node_count.get();
        a.idents += file.identifier_count.get();
        a.symbols += file.symbol_count.get();
        a.parse_ns += rec.parse_ns;
        a.bind_ns += bind_ns;
        a.used += used;
        a.pages += s.pages;
        a.touched += s.touched;
        a.written += s.written;
        a.files_touched += (s.touched > 0) as usize;
        a.grown += ends[i].saturating_sub(rec.cap_at_protect.load(std::sync::atomic::Ordering::Relaxed));
    }
    if let Ok(path) = std::env::var("TSRS_PFE_PROBE_OUT") {
        std::fs::write(path, &out).expect("TSRS_PFE_PROBE_OUT");
    }
    let mib = |b: usize| b as f64 / (1 << 20) as f64;
    eprintln!(
        "pfe: faults {faults}, protect {:.3}s; class files text_MiB lines nodes idents symbols parse_s bind_s used_MiB mapped_pages touched written files_touched grown_MiB",
        pfe_probe::protect_seconds()
    );
    for write in [false, true] {
        let top: Vec<String> = pfe_probe::top_pcs(write, 40).iter().map(|(pc, n)| format!("{pc:#x}:{n}")).collect();
        eprintln!("pfe: first-{} pcs {}", if write { "write" } else { "read" }, top.join(" "));
    }
    let mut classes: Vec<_> = aggs.into_iter().collect();
    classes.sort_by_key(|(c, _)| *c);
    for (c, a) in classes {
        eprintln!(
            "pfe: {c}\t{}\t{:.1}\t{}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{:.1}\t{}\t{}\t{}\t{}\t{:.1}",
            a.files,
            mib(a.text),
            a.lines,
            a.nodes,
            a.idents,
            a.symbols,
            a.parse_ns as f64 * 1e-9,
            a.bind_ns as f64 * 1e-9,
            mib(a.used),
            a.pages,
            a.touched,
            a.written,
            a.files_touched,
            mib(a.grown),
        );
    }
}
