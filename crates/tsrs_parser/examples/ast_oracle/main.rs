// Rust side of the AST oracle; mirror of tools/oracle/ast/main.go (see there for the format).
//
//   ast_oracle dump FILE [FLAGS]   print the AST dump of FILE
//   ast_oracle hash < list         print "hash path" for every "path[\tFLAGS]" line on stdin
//   ast_oracle bench [ROUNDS] < list   parse every listed file (single thread) and report time
//
// Everything runs on a thread with a large stack: Go's goroutine stacks grow, the Rust parser recurses
// on the native stack.

// Same allocator as the tsrs binary, so `bench` measures what tsrs does; `bench` also reports the number of heap
// allocations (arena chunks included).
struct CountingAlloc;

static HEAP_ALLOCS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        HEAP_ALLOCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        mimalloc::MiMalloc.alloc(layout)
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: std::alloc::Layout) {
        mimalloc::MiMalloc.dealloc(p, layout)
    }
    unsafe fn realloc(&self, p: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        HEAP_ALLOCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        mimalloc::MiMalloc.realloc(p, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

mod fields;

use std::fmt::Write as _;
use std::io::{BufRead, Write as _};

use tsrs_ast::{set_external_module_indicator, Diagnostic, ExternalModuleIndicatorOptions, FileReference, Kind, ModifierList, Node, NodeList, SourceFile, SourceFileParseOptions, TokenFlags};
use tsrs_core::tspath::Path;
use tsrs_core::P;

pub(crate) struct Dumper {
    sb: String,
    file: &'static SourceFile,
}

fn escape(sb: &mut String, s: &str) {
    sb.push('"');
    for &b in s.as_bytes() {
        if (0x20..0x7f).contains(&b) && b != b'\\' && b != b'"' {
            sb.push(b as char);
        } else {
            let _ = write!(sb, "\\x{:02x}", b);
        }
    }
    sb.push('"');
}

fn kind_name(k: Kind) -> String {
    format!("{:?}", k)
}

pub(crate) trait ListArg {
    fn opt(self) -> Option<P<NodeList>>;
}
impl ListArg for P<NodeList> {
    fn opt(self) -> Option<P<NodeList>> {
        Some(self)
    }
}
impl ListArg for Option<P<NodeList>> {
    fn opt(self) -> Option<P<NodeList>> {
        self
    }
}

impl Dumper {
    fn node_ref(&self, n: Option<P<Node>>) -> String {
        match n {
            None => "-".to_string(),
            Some(n) if n == self.file.as_node() => "self".to_string(),
            Some(n) => format!("{}@{}:{}", kind_name(n.kind()), n.pos(), n.end()),
        }
    }

    fn name(&mut self, name: &str) {
        self.sb.push(' ');
        self.sb.push_str(name);
    }

    pub(crate) fn field_list(&mut self, name: &str, l: impl ListArg) {
        self.name(name);
        match l.opt() {
            None => self.sb.push('-'),
            Some(l) => {
                let _ = write!(self.sb, "[{},{},{}]", l.pos(), l.end(), l.nodes().len());
            }
        }
    }

    pub(crate) fn field_modlist(&mut self, name: &str, l: Option<P<ModifierList>>) {
        self.name(name);
        match l {
            None => self.sb.push('-'),
            Some(l) => {
                let _ = write!(self.sb, "[{},{},{},{:x}]", l.pos(), l.end(), l.nodes().len(), l.modifier_flags.bits());
            }
        }
    }

    pub(crate) fn field_rawnodes(&mut self, name: &str, nodes: &[P<Node>]) {
        self.name(name);
        self.sb.push('{');
        for (i, n) in nodes.iter().enumerate() {
            if i > 0 {
                self.sb.push(',');
            }
            let _ = write!(self.sb, "{}@{}:{}", kind_name(n.kind()), n.pos(), n.end());
        }
        self.sb.push('}');
    }

    pub(crate) fn field_rawstrings(&mut self, name: &str, strings: &[&str]) {
        self.name(name);
        self.sb.push('{');
        for (i, s) in strings.iter().enumerate() {
            if i > 0 {
                self.sb.push(',');
            }
            escape(&mut self.sb, s);
        }
        self.sb.push('}');
    }

    pub(crate) fn field_bool(&mut self, name: &str, v: bool) {
        self.name(name);
        self.sb.push_str(if v { "=1" } else { "=0" });
    }

    pub(crate) fn field_string(&mut self, name: &str, v: &str) {
        self.name(name);
        self.sb.push('=');
        escape(&mut self.sb, v);
    }

    pub(crate) fn field_tokenflags(&mut self, name: &str, v: TokenFlags) {
        self.name(name);
        let _ = write!(self.sb, "={:x}", v.bits() as u32);
    }

    pub(crate) fn field_kind(&mut self, name: &str, v: Kind) {
        self.name(name);
        self.sb.push('=');
        self.sb.push_str(&kind_name(v));
    }

    fn node(&mut self, n: P<Node>, depth: usize, parent: Option<P<Node>>, tag: char) {
        self.sb.push(tag);
        let _ = write!(self.sb, "{} {} {} {} {:x} sf={:x}", depth, kind_name(n.kind()), n.pos(), n.end(), n.flags().bits(), n.subtree_facts().bits());
        fields::dump_fields(&n, self);
        if n.parent() != parent {
            let r = self.node_ref(n.parent());
            self.sb.push_str(" P!");
            self.sb.push_str(&r);
        }
        self.sb.push('\n');
        for &j in n.jsdoc(Some(self.file)) {
            self.node(j, depth + 1, Some(n), 'J');
        }
        n.for_each_child(&mut |c| {
            self.node(c, depth + 1, Some(n), 'N');
            false
        });
    }

    fn diagnostics(&mut self, tag: &str, diags: &[P<Diagnostic>]) {
        for diag in diags {
            let _ = write!(self.sb, "{} {} {} {} {} ", tag, diag.code(), diag.category() as i32, diag.pos(), diag.len());
            escape(&mut self.sb, diag.message_text());
            let _ = write!(self.sb, " C{} R", diag.message_chain().len());
            for r in diag.related_information() {
                let _ = write!(self.sb, " {}:{}:{}", r.code(), r.pos(), r.len());
            }
            self.sb.push('\n');
        }
    }

    fn file_refs(&mut self, tag: &str, refs: &[P<FileReference>]) {
        for r in refs {
            let _ = write!(self.sb, "{} {} {} ", tag, r.pos(), r.end());
            escape(&mut self.sb, &r.file_name);
            let _ = writeln!(self.sb, " {} {}", r.resolution_mode.value(), r.preserve);
        }
    }
}

fn parse_flags(flags: &str) -> ExternalModuleIndicatorOptions {
    ExternalModuleIndicatorOptions { jsx: flags.contains('j'), force: flags.contains('f') }
}

fn read(path: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(tsrs_vfs::internal::decode_bytes(bytes))
}

fn parse_options(path: &str, flags: &str) -> SourceFileParseOptions {
    SourceFileParseOptions { file_name: path.to_string(), path: Path::new(path.to_string()), external_module_indicator_options: parse_flags(flags) }
}

fn dump(path: &str, flags: &str) -> Result<String, String> {
    let text = read(path).ok_or_else(|| format!("cannot read {}", path))?;
    let file = tsrs_parser::parse_source_file(parse_options(path, flags), &text, tsrs_core::ensure_script_kind_from_file_name(path));
    let file: &'static SourceFile = file.get();
    let mut d = Dumper { sb: String::new(), file };
    let _ = writeln!(
        d.sb,
        "FILE sk={} lv={} decl={} uri={} ids={} nodes={} texts={}",
        file.script_kind.get() as i32,
        file.language_variant.get() as i32,
        file.is_declaration_file.get(),
        file.uses_uri_style_node_core_modules.get() as i32,
        file.identifier_count.get(),
        file.node_count.get(),
        file.text_count.get()
    );
    d.node(file.as_node(), 0, None, 'N');
    d.diagnostics("D", file.diagnostics());
    d.diagnostics("DJS", file.js_diagnostics());
    d.diagnostics("DJD", file.jsdoc_diagnostics());
    for &imp in file.imports() {
        let r = d.node_ref(Some(imp));
        d.sb.push_str("IMP ");
        d.sb.push_str(&r);
        d.sb.push(' ');
        escape(&mut d.sb, imp.text());
        d.sb.push('\n');
    }
    for &aug in file.module_augmentations.get() {
        let r = d.node_ref(Some(aug));
        d.sb.push_str("AUG ");
        d.sb.push_str(&r);
        d.sb.push(' ');
        escape(&mut d.sb, aug.text());
        d.sb.push('\n');
    }
    for name in file.ambient_module_names.get() {
        d.sb.push_str("AMB ");
        escape(&mut d.sb, name);
        d.sb.push('\n');
    }
    for cd in file.comment_directives.get() {
        let _ = writeln!(d.sb, "CD {} {} {}", cd.loc.pos(), cd.loc.end(), cd.kind as i32);
    }
    for &rc in file.reparsed_clones.get() {
        let r = d.node_ref(Some(rc));
        let _ = writeln!(d.sb, "RC {}", r);
    }
    for p in file.pragmas.get() {
        let _ = write!(d.sb, "PRAGMA {} {} {} {} {}", p.name, p.comment_range.kind as i16, p.comment_range.pos(), p.comment_range.end(), p.comment_range.has_trailing_new_line);
        let mut keys: Vec<&String> = p.args.keys().collect();
        keys.sort();
        for k in keys {
            let a = &p.args[k];
            let _ = write!(d.sb, " {}=({},{},", k, a.text_range.pos(), a.text_range.end());
            escape(&mut d.sb, &a.name);
            d.sb.push(',');
            escape(&mut d.sb, &a.value);
            d.sb.push(')');
        }
        d.sb.push('\n');
    }
    d.file_refs("REF", file.referenced_files.get());
    d.file_refs("TREF", file.type_reference_directives.get());
    d.file_refs("LREF", file.lib_reference_directives.get());
    if let Some(c) = file.check_js_directive.get() {
        let _ = writeln!(d.sb, "CHECKJS {} {} {}", c.enabled, c.range.pos(), c.range.end());
    }
    let r = d.node_ref(file.common_js_module_indicator.get());
    let _ = writeln!(d.sb, "CJS {}", r);
    let r = d.node_ref(file.external_module_indicator.get());
    let _ = writeln!(d.sb, "EMI {}", r);
    set_external_module_indicator(file, ExternalModuleIndicatorOptions { jsx: true, force: false });
    let r = d.node_ref(file.external_module_indicator.get());
    let _ = writeln!(d.sb, "EMI_J {}", r);
    set_external_module_indicator(file, ExternalModuleIndicatorOptions { jsx: false, force: true });
    let r = d.node_ref(file.external_module_indicator.get());
    let _ = writeln!(d.sb, "EMI_F {}", r);
    Ok(d.sb)
}

fn fnv64a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn split_line(line: &str) -> (&str, &str) {
    match line.split_once('\t') {
        Some((p, f)) if !f.is_empty() => (p, f),
        Some((p, _)) => (p, "-"),
        None => (line, "-"),
    }
}

fn run() {
    let args: Vec<String> = std::env::args().collect();
    match args[1].as_str() {
        "dump" => {
            let flags = args.get(3).map(|s| s.as_str()).unwrap_or("-");
            match dump(&args[2], flags) {
                Ok(out) => {
                    std::io::stdout().write_all(out.as_bytes()).unwrap();
                }
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1);
                }
            }
        }
        "hash" => {
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let mut w = std::io::BufWriter::new(stdout.lock());
            for line in stdin.lock().lines() {
                let line = line.unwrap();
                let (path, flags) = split_line(&line);
                match dump(path, flags) {
                    Ok(out) => {
                        let _ = writeln!(w, "{:016x} {}", fnv64a(out.as_bytes()), path);
                    }
                    Err(e) => eprintln!("{}", e),
                }
                let _ = w.flush();
            }
        }
        "bench" => {
            let rounds: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            let mut inputs = Vec::new();
            let mut total = 0usize;
            for line in std::io::stdin().lock().lines() {
                let line = line.unwrap();
                let (path, flags) = split_line(&line);
                let Some(text) = read(path) else { continue };
                total += text.len();
                inputs.push((path.to_string(), text, parse_options(path, flags)));
            }
            for _ in 0..rounds {
                let (start_instructions, start_cycles) = process_counters();
                let start_allocs = HEAP_ALLOCS.load(std::sync::atomic::Ordering::Relaxed);
                let start = std::time::Instant::now();
                let mut nodes = 0usize;
                for (path, text, opts) in &inputs {
                    let f = tsrs_parser::parse_source_file(opts.clone(), text, tsrs_core::ensure_script_kind_from_file_name(path));
                    nodes += f.node_count.get();
                }
                let el = start.elapsed().as_secs_f64();
                let (end_instructions, end_cycles) = process_counters();
                let (instructions, cycles) = (end_instructions - start_instructions, end_cycles - start_cycles);
                let allocs = HEAP_ALLOCS.load(std::sync::atomic::Ordering::Relaxed) - start_allocs;
                println!(
                    "rust: {} files, {:.1} MB, {} nodes, {:.3} s, {:.1} MB/s, {:.3} G instructions, {:.3} G cycles, {} heap allocations",
                    inputs.len(),
                    total as f64 / 1e6,
                    nodes,
                    el,
                    total as f64 / 1e6 / el,
                    instructions as f64 / 1e9,
                    cycles as f64 / 1e9,
                    allocs
                );
            }
        }
        other => panic!("unknown command {}", other),
    }
}

/// (instructions retired, cycles) of the whole process so far (user and kernel; macOS `proc_pid_rusage`, zeros
/// elsewhere).
#[cfg(target_os = "macos")]
fn process_counters() -> (u64, u64) {
    extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut u64) -> i32;
    }
    // struct rusage_info_v4 is 41 u64 words after the 16-byte uuid; ri_instructions and ri_cycles are words 31, 32.
    let mut buf = [0u64; 64];
    // SAFETY: the buffer is larger than struct rusage_info_v4 (flavor 4).
    let rc = unsafe { proc_pid_rusage(std::process::id() as i32, 4, buf.as_mut_ptr()) };
    if rc == 0 {
        (buf[31], buf[32])
    } else {
        (0, 0)
    }
}

#[cfg(not(target_os = "macos"))]
fn process_counters() -> (u64, u64) {
    (0, 0)
}

fn main() {
    // TSRS_ORACLE_STACK_MB overrides the stack size, to measure the parser's stack use per nesting level.
    let stack_mb: usize = std::env::var("TSRS_ORACLE_STACK_MB").ok().and_then(|s| s.parse().ok()).unwrap_or(4096);
    let child = std::thread::Builder::new().stack_size(stack_mb << 20).spawn(run).unwrap();
    if child.join().is_err() {
        std::process::exit(101);
    }
}
