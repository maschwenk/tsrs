//! Experiment only (branch exp/crossfile-diagnostics, not for merge).
//!
//! `TSRS_XFILE=<out.tsv>` (build with `--features tsrs_cli/xfile-stats`): counts the diagnostics a checker files against
//! a file other than the one it is checking, and the node-builder work (arena bytes, heap bytes, instructions) spent in
//! `type_to_string` & co. and in reporting relations whose error node lies in such a file. Written at exit.
//!
//! `TSRS_XFILE_SKIP=lost|all` (any build): a reporting relation whose error node lies in another file runs without
//! reporting when this checker's diagnostics for that file are never read (`lost`: the file is already checked here,
//! or a static assignment gives it to another checker) or always (`all`, an upper bound that can change output).

use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::fmt::Write as _;
use std::panic::Location;
use std::sync::{Mutex, OnceLock};
use tsrs_ast::{Diagnostic, Node, SourceFile};
use tsrs_core::xfile::{snapshot, COMPILED};
use tsrs_core::P;

use crate::checker::Checker;

fn out_path() -> Option<&'static str> {
    static OUT: OnceLock<Option<String>> = OnceLock::new();
    OUT.get_or_init(|| std::env::var("TSRS_XFILE").ok().filter(|v| !v.is_empty())).as_deref()
}

#[inline]
pub(crate) fn enabled() -> bool {
    COMPILED && out_path().is_some()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipMode {
    Off,
    Lost,
    All,
}

pub(crate) fn skip_mode() -> SkipMode {
    static MODE: OnceLock<SkipMode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("TSRS_XFILE_SKIP").as_deref() {
        Ok("lost") => SkipMode::Lost,
        Ok("all") => SkipMode::All,
        _ => SkipMode::Off,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct AggKey {
    kind: u8, // 0 scope, 1 print, 2 error add, 3 suggestion add
    checker: u32,
    cross: u8, // 0 same file, 1 other file, 2 no checking file
    x_file: Option<P<SourceFile>>,
    x_state: u8, // 0 not yet checked here, 1 already checked here
    site: &'static Location<'static>,
    code: i32,
    flag: bool, // scope: produced a diagnostic; print: inside a reporting relation
}

#[derive(Default)]
struct Agg {
    n: u64,
    total: [u64; 3],
    print: [u64; 3],
}

struct Scope {
    x_file: Option<P<SourceFile>>,
    cross: u8,
    x_state: u8,
    site: &'static Location<'static>,
    checker: u32,
    start: [u64; 3],
    print: [u64; 3],
    diags: u64,
}

#[derive(Default)]
struct State {
    print_depth: u32,
    print_start: [u64; 3],
    print_target: Option<(u32, u8, Option<P<SourceFile>>, u8, &'static Location<'static>, bool)>,
    scopes: Vec<Scope>,
    diags: u64,
    agg: FxHashMap<AggKey, Agg>,
    rows: Vec<String>,
    seq: FxHashMap<u32, u32>,
}

impl Drop for State {
    fn drop(&mut self) {
        flush(self);
    }
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State::default());
}

static OUT_ROWS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static BT_SEEN: Mutex<Option<FxHashMap<(i32, &'static Location<'static>), u32>>> = Mutex::new(None);

fn file_name(f: Option<P<SourceFile>>) -> String {
    f.map_or_else(|| "-".to_string(), |f| f.file_name().to_string())
}

fn flush(st: &mut State) {
    let mut out = std::mem::take(&mut st.rows);
    for (k, a) in st.agg.drain() {
        let mut line = String::new();
        let _ = write!(
            line,
            "A\t{}\t{}\t{}\t{}\t{}\t{}:{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            k.kind,
            k.checker,
            k.cross,
            file_name(k.x_file),
            k.x_state,
            k.site.file(),
            k.site.line(),
            k.code,
            u8::from(k.flag),
            a.n,
            a.total[0],
            a.total[1],
            a.total[2],
            a.print[0],
            a.print[1],
            a.print[2]
        );
        out.push(line);
    }
    if !out.is_empty() {
        OUT_ROWS.lock().unwrap().extend(out);
    }
}

extern "C" fn write_at_exit() {
    let _ = ST.try_with(|st| flush(&mut st.borrow_mut()));
    let Some(path) = out_path() else { return };
    let rows = std::mem::take(&mut *OUT_ROWS.lock().unwrap());
    let mut s = String::with_capacity(rows.iter().map(|r| r.len() + 1).sum());
    for r in rows {
        s.push_str(&r);
        s.push('\n');
    }
    let _ = std::fs::write(path, s);
}

fn register_exit() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        extern "C" {
            fn atexit(f: extern "C" fn()) -> i32;
        }
        // SAFETY: registers a plain function to run at process exit.
        unsafe { atexit(write_at_exit) };
    });
}

fn x_state(c: &Checker, x: Option<P<SourceFile>>) -> u8 {
    match x {
        Some(f) if c.source_file_links.try_get(f).is_some_and(|l| l.type_checked.get()) => 1,
        _ => 0,
    }
}

fn cross_of(c: &Checker, x: Option<P<SourceFile>>) -> u8 {
    match c.checking_file {
        None => 2,
        Some(y) if Some(y) == x => 0,
        Some(_) => 1,
    }
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn flatten(d: P<Diagnostic>, depth: usize, out: &mut String) {
    if depth > 0 {
        out.push('\n');
        for _ in 0..depth {
            out.push_str("  ");
        }
    }
    out.push_str(&d.localize());
    for &c in d.message_chain() {
        flatten(c, depth + 1, out);
    }
}

pub(crate) fn diagnostic_text(d: P<Diagnostic>) -> String {
    let mut s = String::new();
    flatten(d, 0, &mut s);
    s
}

/// A diagnostic reached the checker's collection (`add_diagnostic` / `add_suggestion_diagnostic`).
pub(crate) fn on_add(c: &Checker, d: P<Diagnostic>, suggestion: bool, site: &'static Location<'static>) {
    if !enabled() {
        return;
    }
    register_exit();
    let x = d.file();
    if x.is_none() {
        return;
    }
    let cross = cross_of(c, x);
    let xs = x_state(c, x);
    let code = d.code();
    let bt = cross != 0 && {
        let mut g = BT_SEEN.lock().unwrap();
        let n = g.get_or_insert_with(FxHashMap::default).entry((code, site)).or_insert(0);
        *n += 1;
        *n <= 2
    };
    let bt = bt.then(|| std::backtrace::Backtrace::force_capture().to_string());
    ST.with(|st| {
        let mut st = st.borrow_mut();
        st.diags += 1;
        let key = AggKey {
            kind: if suggestion { 3 } else { 2 },
            checker: c.id,
            cross,
            x_file: if cross == 1 { x } else { None },
            x_state: xs,
            site,
            code,
            flag: !st.scopes.is_empty(),
        };
        st.agg.entry(key).or_default().n += 1;
        if cross == 1 {
            let scope_site = st.scopes.first().map_or_else(|| "-".to_string(), |s| format!("{}:{}", s.site.file(), s.site.line()));
            let text = diagnostic_text(d);
            let seq = st.seq.get(&c.id).copied().unwrap_or(0);
            let row = format!(
                "D\t{}\t{}\t{}\t{}\t{}\t{}\t{}:{}\t{}\t{}\t{}\t{}",
                c.id,
                seq,
                xs,
                u8::from(suggestion),
                code,
                file_name(x),
                site.file(),
                site.line(),
                scope_site,
                d.pos(),
                file_name(c.checking_file),
                esc(&text)
            );
            st.rows.push(row);
        }
        if let Some(bt) = bt {
            let pat = if cross == 1 { "tsrs_checker::" } else { "tsrs_" };
            let frames: Vec<&str> = bt
                .lines()
                .map(str::trim)
                .filter(|l| l.contains(pat) && !l.contains("xfile"))
                .map(|l| l.split_once(": ").map_or(l, |(_, f)| f))
                .collect();
            st.rows.push(format!("B\t{}\t{}:{}\t{}\t{}", code, site.file(), site.line(), cross, esc(&frames.join(" <- "))));
        }
    });
}

/// A file's diagnostics were collected (the program's output for that file).
pub(crate) fn on_collect(c: &Checker, file: P<SourceFile>, diags: &[P<Diagnostic>], suggestions: bool) {
    if !enabled() {
        return;
    }
    register_exit();
    ST.with(|st| {
        let mut st = st.borrow_mut();
        let seq = {
            let s = st.seq.entry(c.id).or_insert(0);
            *s += 1;
            *s
        };
        st.rows.push(format!("C\t{}\t{}\t{}\t{}", c.id, seq, file_name(Some(file)), u8::from(suggestions)));
        for &d in diags {
            let row = format!("F\t{}\t{}\t{}\t{}", file_name(d.file()), d.pos(), d.code(), esc(&diagnostic_text(d)));
            st.rows.push(row);
        }
    });
}

/// `report_diagnostic` with a `diagnostic_output`: counts as a produced diagnostic for the enclosing scope.
pub(crate) fn on_output_push() {
    if enabled() {
        ST.with(|st| st.borrow_mut().diags += 1);
    }
}

pub(crate) struct PrintGuard(bool);

#[track_caller]
pub(crate) fn print_enter(c: &Checker) -> PrintGuard {
    if !enabled() {
        return PrintGuard(false);
    }
    let site = Location::caller();
    ST.with(|st| {
        let mut st = st.borrow_mut();
        st.print_depth += 1;
        if st.print_depth == 1 {
            let target = if let Some(s) = st.scopes.first() {
                (s.checker, s.cross, s.x_file, s.x_state, site, true)
            } else {
                let x = c.current_node.and_then(|n| tsrs_ast::get_source_file_of_node(Some(n)));
                let cross = cross_of(c, x);
                (c.id, cross, if cross == 1 { x } else { None }, x_state(c, x), site, false)
            };
            st.print_target = Some(target);
            st.print_start = snapshot();
        }
    });
    PrintGuard(true)
}

impl Drop for PrintGuard {
    fn drop(&mut self) {
        if !self.0 {
            return;
        }
        ST.with(|st| {
            let mut st = st.borrow_mut();
            st.print_depth -= 1;
            if st.print_depth == 0 {
                let now = snapshot();
                let d = [now[0] - st.print_start[0], now[1] - st.print_start[1], now[2] - st.print_start[2]];
                if let Some(s) = st.scopes.first_mut() {
                    for i in 0..3 {
                        s.print[i] += d[i];
                    }
                }
                let (checker, cross, x_file, x_state, site, in_scope) = st.print_target.take().unwrap();
                let key = AggKey { kind: 1, checker, cross, x_file, x_state, site, code: 0, flag: in_scope };
                let a = st.agg.entry(key).or_default();
                a.n += 1;
                for i in 0..3 {
                    a.total[i] += d[i];
                    a.print[i] += d[i];
                }
            }
        });
    }
}

pub(crate) struct ScopeGuard(bool);

#[track_caller]
pub(crate) fn scope_enter(c: &Checker, error_node: Option<P<Node>>) -> ScopeGuard {
    if !enabled() || error_node.is_none() {
        return ScopeGuard(false);
    }
    let site = Location::caller();
    let x = tsrs_ast::get_source_file_of_node(error_node);
    let cross = cross_of(c, x);
    let xs = x_state(c, x);
    ST.with(|st| {
        let mut st = st.borrow_mut();
        let diags = st.diags;
        let start = if st.scopes.is_empty() { snapshot() } else { [0; 3] };
        st.scopes.push(Scope {
            x_file: if cross == 1 { x } else { None },
            cross,
            x_state: xs,
            site,
            checker: c.id,
            start,
            print: [0; 3],
            diags,
        });
    });
    ScopeGuard(true)
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        if !self.0 {
            return;
        }
        ST.with(|st| {
            let mut st = st.borrow_mut();
            let s = st.scopes.pop().unwrap();
            if st.scopes.is_empty() {
                let now = snapshot();
                let d = [now[0] - s.start[0], now[1] - s.start[1], now[2] - s.start[2]];
                let produced = st.diags > s.diags;
                let key = AggKey { kind: 0, checker: s.checker, cross: s.cross, x_file: s.x_file, x_state: s.x_state, site: s.site, code: 0, flag: produced };
                let a = st.agg.entry(key).or_default();
                a.n += 1;
                for i in 0..3 {
                    a.total[i] += d[i];
                    a.print[i] += s.print[i];
                }
            }
        });
    }
}

/// `TSRS_XFILE_SKIP`: whether a reporting relation whose error node is `error_node` should run without reporting.
pub(crate) fn skip_reporting(c: &Checker, error_node: Option<P<Node>>) -> bool {
    let mode = skip_mode();
    if mode == SkipMode::Off {
        return false;
    }
    let Some(y) = c.checking_file else { return false };
    let Some(x) = tsrs_ast::get_source_file_of_node(error_node) else { return false };
    if x == y {
        return false;
    }
    match mode {
        SkipMode::All => true,
        SkipMode::Lost => {
            x_state(c, Some(x)) == 1 || c.xfile_owned.as_ref().is_some_and(|owned: &FxHashSet<P<SourceFile>>| !owned.contains(&x))
        }
        SkipMode::Off => false,
    }
}
