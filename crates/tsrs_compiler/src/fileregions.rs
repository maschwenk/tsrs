//! tsrs-only: per-file regions in the CLI's `--noEmit` check, and freeing the tree and binder output of a checked
//! "leaf" file as soon as its checker is done with it (notes/mem-free-leaf-files.md, after Bun's `free_tree`).
//!
//! When the CLI turns file regions on (`enable`, before it creates the program), the host parses each TypeScript
//! source file (`.ts`, `.tsx`, `.mts`, `.cts`; not declaration files) into a scratch region of its own, and the file
//! is bound in that region too (`bind`). The text, the `SourceFile` itself and the diagnostics stay outside it (the
//! parser and the diagnostics escape the scratch region), so a freed file still has what the report reads: name,
//! text, line map, counters, and its diagnostics, collected before the free.
//!
//! A program created with `ProgramOptions::leaf_files` set marks its leaves right before its type-check pass
//! (`classify`, `SourceFile::is_check_leaf`): type-checked TypeScript modules that no other file refers to and that
//! declare nothing another file can reach. After a leaf's semantic diagnostics are collected (the pass's callback),
//! `free` drops its region with `Region::retire_on_free`: its pages go back to the system and its address range is
//! never reused. Every other region lives for the rest of the process, as the thread arenas did.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{self as ast, SourceFile, SourceFileParseOptions, SymbolFlags};
use tsrs_core::arena::Region;
use tsrs_core::{ScriptKind, P};

use crate::program::Program;

/// What a program does with its file regions (`ProgramOptions::leaf_files`; `TSRS_FREE_LEAVES`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LeafMode {
    /// Nothing (the default; `TSRS_FREE_LEAVES=0`, and everywhere but the CLI's `--noEmit` check).
    #[default]
    Off,
    /// Leaves are freed once checked (the CLI's default where allowed).
    Free,
    /// Leaves are counted, nothing is freed (`TSRS_FREE_LEAVES=keep`: measures what the regions cost).
    Keep,
}

static REGIONS_ON: AtomicBool = AtomicBool::new(false);
static STATS: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The host is parsing a root file of the program (`parse_task`).
    static ROOT_PARSE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// File regions by file. Filled by the parse workers, read by the binding ones, emptied of leaves by the checkers.
static REGIONS: LazyLock<Mutex<FxHashMap<P<SourceFile>, Region>>> = LazyLock::new(Default::default);

// `TSRS_FREE_LEAVES=stats` / `keep` counters: written before and during the pass, read after its threads joined.
static LEAVES: AtomicUsize = AtomicUsize::new(0);
static LEAF_BYTES: AtomicUsize = AtomicUsize::new(0);
static FREED: AtomicUsize = AtomicUsize::new(0);
static FREED_BYTES: AtomicUsize = AtomicUsize::new(0);
static REGION_FILES: AtomicUsize = AtomicUsize::new(0);
static REGION_BYTES: AtomicUsize = AtomicUsize::new(0);
static REGION_USED: AtomicUsize = AtomicUsize::new(0);
static CHECKED_FILES: AtomicUsize = AtomicUsize::new(0);

/// The mode `TSRS_FREE_LEAVES` asks for where freeing is allowed, and whether to report (`stats_report`): unset or
/// `1` frees, `stats` frees and reports, `keep` makes the regions and reports but frees nothing, `0` turns it off.
/// Off under the debug modes that walk files or checker data after the pass (`TSRS_FILE_TIMES` walks every tree;
/// `TSRS_ASSIGNMENT_STATS`, the work and heap censuses walk checker tables) or that must see every block alive (the
/// reachability census, `TSRS_CENSUS=1`).
pub fn leaf_mode_from_env() -> (LeafMode, bool) {
    let census = std::env::var_os("TSRS_CENSUS").is_some_and(|v| v == "1") || tsrs_core::census_recording();
    #[cfg(feature = "checker")]
    let checker_census = crate::Checker::census_enabled() || crate::Checker::heap_census_enabled();
    #[cfg(not(feature = "checker"))]
    let checker_census = false;
    if census || checker_census || crate::checkerpool::file_times_path().is_some() || crate::checkerpool::assignment_stats_enabled() {
        return (LeafMode::Off, false);
    }
    match std::env::var("TSRS_FREE_LEAVES").as_deref() {
        Ok("0") => (LeafMode::Off, false),
        Ok("stats") => (LeafMode::Free, true),
        Ok("keep") => (LeafMode::Keep, true),
        _ => (LeafMode::Free, false),
    }
}

/// Turns file regions on for the programs created from here on, process-wide (the CLI, before `new_program`; never
/// the language server, the API or the test harnesses). `stats`: `stats_report` reports.
pub fn enable(stats: bool) {
    // Relaxed (both): set on the main thread before the program, and any worker that reads them, exists.
    REGIONS_ON.store(true, Ordering::Relaxed);
    STATS.store(stats, Ordering::Relaxed);
}

#[inline]
fn stats() -> bool {
    // Relaxed: see `enable`.
    STATS.load(Ordering::Relaxed)
}

/// Runs `parse` (the host's `get_source_file` for a task of the loader) knowing whether the task is a root file of the
/// program (`root`, its include reason): only a file parsed for one can be a leaf, as a file parsed for an import or a
/// reference has a referrer, so only it gets a region (`wants_region`). In a monorepo app most files come from other
/// packages through imports (cal-diy: 2,643 of 3,550 files), and their regions would cost and free nothing.
pub(crate) fn parse_task<T>(root: bool, parse: impl FnOnce() -> T) -> T {
    // Relaxed: see `enable`.
    if !root || !REGIONS_ON.load(Ordering::Relaxed) {
        return parse();
    }
    let saved = ROOT_PARSE.replace(true);
    let file = parse();
    ROOT_PARSE.set(saved);
    file
}

/// Whether a file of this name and kind is parsed into a region of its own: only files that can be leaves, TypeScript
/// files (not declaration files) parsed for a root file of the program (`parse_task`).
pub(crate) fn wants_region(file_name: &str, script_kind: ScriptKind) -> bool {
    // Relaxed: see `enable`.
    REGIONS_ON.load(Ordering::Relaxed)
        && ROOT_PARSE.get()
        && matches!(script_kind, ScriptKind::TS | ScriptKind::TSX)
        && !tsrs_core::tspath::is_declaration_file_name(file_name)
}

// Text, AST and binder data take about 8 times the text (tsrs_project parsecache.rs measured 7.8x on the private
// monorepo); the region grows by a quarter when this is exceeded and is trimmed to what it used.
fn first_chunk(text_len: usize) -> usize {
    text_len.saturating_mul(10).saturating_add(4 << 10).min(64 << 20)
}

/// Parses `text` (just read) into a fresh region (`wants_region`). The text itself is kept outside it. The region is
/// trimmed right away: binding may come much later (single-threaded loads bind every file at the end), and the next
/// file's region is then carved right after this one's last used byte instead of after an unused tail and its partly
/// used page. Binding on the same thread right away (the parallel loader) grows the region into the space just given
/// back.
pub(crate) fn parse(opts: SourceFileParseOptions, text: String, script_kind: ScriptKind) -> P<SourceFile> {
    let region = Region::new_scratch(first_chunk(text.len()));
    let file = {
        let _scratch = region.enter_scratch();
        tsrs_parser::parse_source_file_keep_text(opts, text, script_kind)
    };
    region.trim();
    REGIONS.lock().unwrap().insert(file, region);
    file
}

/// `tsrs_binder::bind_source_file`, in the file's region if it has one (then trimmed to what parse and bind used, if
/// this thread carved it last).
pub(crate) fn bind(file: P<SourceFile>) {
    // Relaxed: see `enable`.
    if file.is_bound() || !REGIONS_ON.load(Ordering::Relaxed) {
        tsrs_binder::bind_source_file(file);
        return;
    }
    let Some(region) = REGIONS.lock().unwrap().get(&file).cloned() else {
        tsrs_binder::bind_source_file(file);
        return;
    };
    {
        let _scratch = region.enter_scratch();
        tsrs_binder::bind_source_file(file);
    }
    region.trim();
}

/// Marks the leaves among `program`'s files before its type-check pass (`SourceFile::set_check_leaf`) when its
/// `leaf_files` mode asks for it, and returns whether the pass frees them.
pub(crate) fn classify(program: &Program) -> bool {
    let mode = program.leaf_files;
    if mode == LeafMode::Off {
        return false;
    }
    // A second pass would read the freed files (their bind diagnostics and comment directives). The CLI runs one.
    // Relaxed: only the thread that runs the program's passes reads and writes it.
    assert!(
        mode != LeafMode::Free || !program.leaf_pass_started.swap(true, Ordering::Relaxed),
        "fileregions: a second type-check pass after leaf files were freed"
    );
    // The test reads the binder's output (the module symbol's exports, UMD globals, pattern ambient modules). The
    // CLI has bound every file by now (its bind diagnostics come first); creating the checkers would bind them next
    // anyway, in the same order.
    program.bind_source_files();
    let files = program.files;
    let regions = REGIONS.lock().unwrap();
    let referred = referred_files(program);
    let (mut leaves, mut leaf_bytes, mut checked) = (0, 0, 0);
    for &file in files {
        if !program.skip_type_checking(file, false) {
            checked += 1;
        }
        let Some(region) = regions.get(&file) else { continue };
        if referred.contains(&file) || !adds_nothing(program, file) {
            continue;
        }
        file.set_check_leaf(true);
        leaves += 1;
        if stats() {
            leaf_bytes += region.used_bytes();
        }
    }
    #[expect(clippy::iter_over_hash_type, reason = "independent per region; no output depends on the order")]
    for (file, region) in regions.iter() {
        // Never freed (also the regions of files parsed ahead and not used): their values live for the process, as
        // in a thread arena, so the list of the ones to drop (24 bytes per symbol table, the one heap-owning value
        // parse and bind make) goes now, before the check pass.
        if !file.is_check_leaf() {
            region.forget_drops();
        }
    }
    if stats() {
        let region_bytes = regions.values().map(Region::allocated_bytes).sum();
        let region_used = regions.values().map(Region::used_bytes).sum();
        for (counter, value) in [
            (&LEAVES, leaves),
            (&LEAF_BYTES, leaf_bytes),
            (&CHECKED_FILES, checked),
            (&REGION_FILES, regions.len()),
            (&REGION_BYTES, region_bytes),
            (&REGION_USED, region_used),
        ] {
            // Relaxed: read on the main thread after the pass's threads joined.
            counter.store(value, Ordering::Relaxed);
        }
    }
    mode == LeafMode::Free
}

/// Files that another file of the program refers to: every file with an include reason other than being a root
/// (an import, a `/// <reference path>`, a type reference directive, a lib reference, a default lib, an automatic
/// type directive), and every target of a resolved import, module augmentation or type reference directive of
/// another file. A file that refers only to itself is not counted.
fn referred_files(program: &Program) -> FxHashSet<P<SourceFile>> {
    let mut referred = FxHashSet::default();
    #[expect(clippy::iter_over_hash_type, reason = "builds a set; the order of insertion cannot be seen")]
    for (path, reasons) in &program.file_include_data.file_include_reasons {
        if reasons.iter().any(|r| !r.is_root_file()) {
            if let Some(&file) = program.files_by_path.get(path) {
                referred.insert(file);
            }
        }
    }
    // The targets of every importer's resolutions. Looking a resolved file name up normalizes it (vscode: 110k
    // imports, 24 ms on one thread, all of it inside the check time), so this runs on the worker pool, like the
    // import graph of the checker assignment (checkerpool.rs `get_import_adjacency`).
    let mut importers: Vec<(&tsrs_core::tspath::Path, Vec<&'static str>)> = Vec::new();
    #[expect(clippy::iter_over_hash_type, reason = "builds a set from them; the order cannot be seen")]
    for (path, resolutions) in &program.resolved_modules {
        importers.push((path, resolutions.values().filter(|r| r.is_resolved()).map(|r| r.resolved_file_name).collect()));
    }
    #[expect(clippy::iter_over_hash_type, reason = "builds a set from them; the order cannot be seen")]
    for (path, resolutions) in &program.type_resolutions_in_file {
        importers.push((path, resolutions.values().filter(|r| r.is_resolved()).map(|r| r.resolved_file_name).collect()));
    }
    let targets_of = |(path, names): &(&tsrs_core::tspath::Path, Vec<&'static str>)| -> Vec<P<SourceFile>> {
        let from = program.files_by_path.get(*path).copied();
        names.iter().filter_map(|name| program.get_source_file_for_resolved_module(name)).filter(|&target| Some(target) != from).collect()
    };
    let targets: Vec<Vec<P<SourceFile>>> = if program.single_threaded() {
        importers.iter().map(targets_of).collect()
    } else {
        use rayon::prelude::*;
        crate::program::worker_pool().install(|| importers.par_iter().map(targets_of).collect())
    };
    referred.extend(targets.into_iter().flatten());
    referred
}

/// Whether nothing of `file` can be reached from another file that does not refer to it. It is a type-checked,
/// non-declaration TypeScript external module (its top-level declarations are module scoped; JavaScript is excluded
/// because CommonJS exports and expando assignments can declare globals), with no module augmentation (including
/// `declare global`), no ambient module, no pattern ambient module, no UMD global (`export as namespace`), and no
/// `// @ts-check` directive (read through the file by `skip_type_checking`). And it exports no alias (`export ...
/// from`, `export *`, `export =`, `export { local }`, `export default <name>`): the checker's whole-program loops
/// (`getAlternativeContainingModules`) ask every module whether it re-exports a symbol, and a module whose exports
/// are its own declarations answers no for any symbol declared elsewhere, so they can skip it
/// (`Checker::is_unreadable_check_leaf`).
fn adds_nothing(program: &Program, file: P<SourceFile>) -> bool {
    if program.skip_type_checking(file, false)
        || file.is_declaration_file.get()
        || !matches!(file.script_kind.get(), ScriptKind::TS | ScriptKind::TSX)
        || !ast::is_external_module(file)
        || !file.module_augmentations.get().is_empty()
        || !file.ambient_module_names.get().is_empty()
        || !file.pattern_ambient_modules.get().is_empty()
        || file.global_exports.get().is_some_and(|g| !g.is_empty())
        || file.check_js_directive.get().is_some()
    {
        return false;
    }
    let Some(symbol) = file.symbol() else { return false };
    let Some(exports) = symbol.exports() else { return true };
    exports.entries().iter().all(|&(name, s)| {
        name != ast::InternalSymbolNameExportEquals
            && name != ast::InternalSymbolNameExportStar
            && !s.flags().intersects(SymbolFlags::Alias | SymbolFlags::ExportStar)
    })
}

/// After the type-check pass that freed leaves: gives back the pages of the last batch of freed regions.
pub(crate) fn pass_done() {
    tsrs_core::arena::flush_retired();
}

/// Frees `file`'s region (a leaf whose diagnostics the pass has collected). Called on the checker thread that
/// checked it; nothing reads the file's tree or binder output afterwards (`classify`).
pub(crate) fn free(file: P<SourceFile>) {
    let Some(region) = REGIONS.lock().unwrap().remove(&file) else { return };
    if stats() {
        // Relaxed: counters read after the pass's threads joined.
        FREED.fetch_add(1, Ordering::Relaxed);
        FREED_BYTES.fetch_add(region.allocated_bytes(), Ordering::Relaxed);
    }
    region.retire_on_free();
    drop(region);
}

/// The `TSRS_FREE_LEAVES=stats` / `keep` line (for stderr), if asked for.
pub fn stats_report() -> Option<String> {
    if !stats() {
        return None;
    }
    let mb = |b: usize| b as f64 / (1 << 20) as f64;
    // Relaxed (all loads): read on the main thread after the pass's threads joined.
    let load = |c: &AtomicUsize| c.load(Ordering::Relaxed);
    let reserve = tsrs_core::ptr::reserve_stats().map(|(_, high)| format!("; arena address space used {:.1} MB", mb(high))).unwrap_or_default();
    let (calls, bytes) = tsrs_core::arena::retired_stats();
    Some(format!(
        "tsrs: leaf files: {} of {} checked files, {:.1} MB of the {:.1} MB used by {} file regions ({:.1} MB reserved); freed {} ({:.1} MB reserved, {:.1} MB of pages given back in {calls} calls){reserve}\n",
        load(&LEAVES),
        load(&CHECKED_FILES),
        mb(load(&LEAF_BYTES)),
        mb(load(&REGION_USED)),
        load(&REGION_FILES),
        mb(load(&REGION_BYTES)),
        load(&FREED),
        mb(load(&FREED_BYTES)),
        mb(bytes),
    ))
}
