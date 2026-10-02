// TSRS_ASSIGNMENT_STATS=dup (feature `assignment-stats`): what the checkers of a multi-checker run created more
// than once (tsrs_checker::dupstats has the fingerprints; notes/mem-shared-base.md the method and the results).

use std::fmt::Write;
use std::sync::Mutex;

use rustc_hash::FxHashMap;
use tsrs_ast::SourceFile;
use tsrs_checker::dupstats::{self, Census, DupRec, LinkKeys, KINDS};
use tsrs_core::P;

use crate::program::Program;

const CAT_NAMES: [&str; 5] = ["none", "lib", "node_modules", "workspace", "project"];

fn file_cat(program: &Program, file: P<SourceFile>, project_dir: &str) -> u8 {
    let name = file.file_name();
    if program.is_source_file_default_library(&file.path()) {
        dupstats::CAT_LIB
    } else if name.contains("/node_modules/") {
        dupstats::CAT_NODE_MODULES
    } else if name.starts_with(project_dir) {
        dupstats::CAT_PROJECT
    } else {
        dupstats::CAT_WORKSPACE
    }
}

#[derive(Default, Clone)]
struct KindTotals {
    count: u64,
    bytes: u64,
    distinct: u64,
    dup_count: u64,
    dup_bytes: u64,
    dup_bytes_full: [u64; 5],
    dup_bytes_decl: [u64; 5],
    dup_bytes_collide: u64,
    dup_bytes_noncanonical: u64,
}

fn mb(b: u64) -> f64 {
    b as f64 / (1u64 << 20) as f64
}

pub(crate) fn report(program: &'static Program, pool: &crate::checkerpool::checkerPool, project_dir: &str) -> String {
    // The project is the tsconfig's directory when there is one (the current directory is wherever tsrs ran).
    let config_path = program.options().config_file_path.clone();
    let project_dir = if config_path.is_empty() {
        project_dir.to_string()
    } else {
        format!("{}/", tsrs_core::tspath::get_directory_path(&config_path).trim_end_matches('/'))
    };
    let project_dir = project_dir.as_str();
    let k = pool.checker_count();
    let recs: Vec<Mutex<Vec<DupRec>>> = (0..k).map(|_| Mutex::new(Vec::new())).collect();
    let links: Vec<Mutex<Vec<LinkKeys>>> = (0..k).map(|_| Mutex::new(Vec::new())).collect();
    let started = std::time::Instant::now();
    pool.for_each_checker_parallel(|idx, c| {
        let cat = |f: P<SourceFile>| file_cat(program, f, project_dir);
        let mut census = Census::new(c, &cat);
        *recs[idx].lock().unwrap() = census.collect();
        *links[idx].lock().unwrap() = census.link_keys();
    });
    let recs: Vec<Vec<DupRec>> = recs.into_iter().map(|r| r.into_inner().unwrap()).collect();
    let links: Vec<Vec<LinkKeys>> = links.into_iter().map(|r| r.into_inner().unwrap()).collect();

    let mut out = String::new();
    let _ = writeln!(out, "\n== cross-checker duplication census ({k} checkers, {:.1}s) ==", started.elapsed().as_secs_f64());
    for (c, r) in recs.iter().enumerate() {
        let bytes: u64 = r.iter().map(|x| x.bytes as u64).sum();
        let _ = writeln!(out, "checker {c}: {} objects, {:.1} MB", r.len(), mb(bytes));
    }

    // Group by fingerprint: (checker, rec) sorted by fp.
    let mut all: Vec<(u128, u8, u32)> = Vec::new(); // fp, checker, index
    for (c, r) in recs.iter().enumerate() {
        for (i, x) in r.iter().enumerate() {
            all.push((x.fp, c as u8, i as u32));
        }
    }
    all.sort_unstable_by_key(|x| (x.0, x.1));
    let mut totals: Vec<KindTotals> = vec![KindTotals::default(); KINDS.len()];
    // Duplicated bytes by the number of checkers holding the fingerprint, and by that number and category.
    let mut dup_by_holders = vec![[0u64; 5]; k + 1];
    // TSRS_DUP_MASKS=<path> (k <= 32): bytes per copy by the set of checkers holding the fingerprint, for offline
    // assignment experiments (notes/mem-shared-base.md).
    let masks_path = std::env::var("TSRS_DUP_MASKS").ok().filter(|_| k <= 32);
    let mut masks: FxHashMap<u32, u64> = FxHashMap::default();
    let mut i = 0;
    while i < all.len() {
        let mut j = i;
        while j < all.len() && all[j].0 == all[i].0 {
            j += 1;
        }
        let group = &all[i..j];
        let mut per = vec![0u64; k];
        let mut bytes = 0u64;
        for &(_, c, idx) in group {
            per[c as usize] += 1;
            bytes += recs[c as usize][idx as usize].bytes as u64;
        }
        let first = recs[group[0].1 as usize][group[0].2 as usize];
        if masks_path.is_some() {
            let mask = per.iter().enumerate().filter(|(_, &n)| n > 0).fold(0u32, |m, (c, _)| m | 1 << c);
            *masks.entry(mask).or_default() += bytes / per.iter().sum::<u64>();
        }
        let n: u64 = per.iter().sum();
        let max = *per.iter().max().unwrap();
        let dup = n - max;
        let t = &mut totals[first.kind as usize];
        t.count += n;
        t.bytes += bytes;
        t.distinct += 1;
        if dup > 0 {
            let dup_bytes = bytes * dup / n;
            t.dup_count += dup;
            t.dup_bytes += dup_bytes;
            t.dup_bytes_full[first.full_cat as usize] += dup_bytes;
            t.dup_bytes_decl[first.decl_cat as usize] += dup_bytes;
            if max > 1 {
                t.dup_bytes_collide += dup_bytes;
            }
            if !first.canonical {
                t.dup_bytes_noncanonical += dup_bytes;
            }
            let holders = per.iter().filter(|&&n| n > 0).count();
            dup_by_holders[holders][first.full_cat as usize] += dup_bytes;
        }
        i = j;
    }

    let _ = writeln!(
        out,
        "{:<32} {:>10} {:>9} {:>10} {:>10} {:>9} | dup MB by most specific declaration involved: {} | by declaring file: {} | collide nonc",
        "kind",
        "count",
        "MB",
        "distinct",
        "dup count",
        "dup MB",
        CAT_NAMES.join("/"),
        CAT_NAMES.join("/")
    );
    let mut grand = KindTotals::default();
    for (kind, t) in totals.iter().enumerate() {
        if t.count == 0 {
            continue;
        }
        let full: Vec<String> = t.dup_bytes_full.iter().map(|&b| format!("{:.1}", mb(b))).collect();
        let decl: Vec<String> = t.dup_bytes_decl.iter().map(|&b| format!("{:.1}", mb(b))).collect();
        let _ = writeln!(
            out,
            "{:<32} {:>10} {:>9.1} {:>10} {:>10} {:>9.1} | {} | {} | {:.1} {:.1}",
            KINDS[kind],
            t.count,
            mb(t.bytes),
            t.distinct,
            t.dup_count,
            mb(t.dup_bytes),
            full.join("/"),
            decl.join("/"),
            mb(t.dup_bytes_collide),
            mb(t.dup_bytes_noncanonical)
        );
        grand.count += t.count;
        grand.bytes += t.bytes;
        grand.distinct += t.distinct;
        grand.dup_count += t.dup_count;
        grand.dup_bytes += t.dup_bytes;
        for c in 0..5 {
            grand.dup_bytes_full[c] += t.dup_bytes_full[c];
            grand.dup_bytes_decl[c] += t.dup_bytes_decl[c];
        }
        grand.dup_bytes_collide += t.dup_bytes_collide;
        grand.dup_bytes_noncanonical += t.dup_bytes_noncanonical;
    }
    let full: Vec<String> = grand.dup_bytes_full.iter().map(|&b| format!("{:.1}", mb(b))).collect();
    let decl: Vec<String> = grand.dup_bytes_decl.iter().map(|&b| format!("{:.1}", mb(b))).collect();
    let _ = writeln!(
        out,
        "{:<32} {:>10} {:>9.1} {:>10} {:>10} {:>9.1} | {} | {} | {:.1} {:.1}",
        "TOTAL",
        grand.count,
        mb(grand.bytes),
        grand.distinct,
        grand.dup_count,
        mb(grand.dup_bytes),
        full.join("/"),
        decl.join("/"),
        mb(grand.dup_bytes_collide),
        mb(grand.dup_bytes_noncanonical)
    );

    if let Some(path) = masks_path {
        let mut text = String::new();
        let mut entries: Vec<(u32, u64)> = masks.into_iter().collect();
        entries.sort_unstable();
        for (mask, bytes) in entries {
            let _ = writeln!(text, "{mask}\t{bytes}");
        }
        let _ = std::fs::write(path, text);
    }
    let _ = writeln!(out, "dup MB by number of checkers holding the fingerprint (by most specific declaration involved {}):", CAT_NAMES.join("/"));
    for (holders, by_cat) in dup_by_holders.iter().enumerate().skip(2) {
        let cats: Vec<String> = by_cat.iter().map(|&b| format!("{:.1}", mb(b))).collect();
        let _ = writeln!(out, "  held by {holders}: {:.1} MB | {}", mb(by_cat.iter().sum()), cats.join("/"));
    }
    // Link stores: records whose key is a shared object (binder symbol, AST node) present in several checkers.
    let _ = writeln!(out, "link stores: records (sum over checkers), on shared keys, duplicated records (sum - distinct shared keys), dup MB; dup MB by key category {}", CAT_NAMES.join("/"));
    let stores = links[0].len();
    let mut link_dup_total = 0u64;
    for s in 0..stores {
        let name = links[0][s].name;
        let record_bytes = links[0][s].record_bytes as u64;
        let mut total = 0u64;
        let mut shared = 0u64;
        let mut seen: FxHashMap<u64, (u32, u8)> = FxHashMap::default();
        for checker_links in &links {
            let store = &checker_links[s];
            total += store.keys.len() as u64;
            for &(key, cat, is_shared) in &store.keys {
                if is_shared {
                    shared += 1;
                    let e = seen.entry(key).or_insert((0, cat));
                    e.0 += 1;
                }
            }
        }
        let mut dup_by_cat = [0u64; 5];
        let mut dup = 0u64;
        for (_, &(n, cat)) in seen.iter() {
            if n > 1 {
                dup += (n - 1) as u64;
                dup_by_cat[cat as usize] += (n - 1) as u64 * record_bytes;
            }
        }
        link_dup_total += dup * record_bytes;
        let cats: Vec<String> = dup_by_cat.iter().map(|&b| format!("{:.1}", mb(b))).collect();
        let _ = writeln!(
            out,
            "  {:<40} {:>10} {:>10} {:>10} {:>8.1} | {}",
            name,
            total,
            shared,
            dup,
            mb(dup * record_bytes),
            cats.join("/")
        );
    }
    let _ = writeln!(out, "  link records on shared keys duplicated: {:.1} MB", mb(link_dup_total));
    // Id-keyed stores: the ids are process-wide, so one checker's ids are spread over the whole id space. Pages a
    // checker would allocate at several page sizes (4 bytes per id).
    for (s, name) in links[0].iter().enumerate().filter(|(_, l)| l.name.contains("by ")).map(|(s, l)| (s, l.name)) {
        let mut line = format!("  pages of {name}:");
        for shift in [10u32, 8, 6, 4] {
            let mut bytes = 0u64;
            for checker_links in &links {
                let mut pages: Vec<u64> = checker_links[s].keys.iter().map(|&(id, _, _)| id >> shift).collect();
                pages.sort_unstable();
                pages.dedup();
                bytes += pages.len() as u64 * (4u64 << shift);
            }
            let _ = write!(line, " {}ids {:.1} MB", 1u64 << shift, mb(bytes));
        }
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out, "project dir {project_dir}");
    out
}
