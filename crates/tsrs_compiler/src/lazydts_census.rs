//! Lazy declaration-file census, program side (alloc-profile builds, `TSRS_LAZY_DTS_CENSUS=1`, with lazy lists off;
//! see `tsrs_core::lazydts_census`, notes/mem-lazy-dts-members.md "Round 2"). `register` runs before the first
//! checker is created: it walks every declaration file the program will not type-check and registers the non-empty
//! member lists of interfaces, classes, type literals and module blocks with their owner symbols. `report` prints,
//! after the check, the bytes that lists nobody asked for would save under the design of #202 and under its
//! extensions (module blocks, lists with import types, lists with eager JSDoc).

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{self as ast, Kind, Node, NodeList, Symbol};
use tsrs_core::lazydts_census as census;
use tsrs_core::P;

use crate::program::Program;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum ListKind {
    Interface,
    Class,
    TypeLiteral,
    Namespace,
    AmbientModule,
    Augmentation,
    Global,
}

const KINDS: [ListKind; 7] = [
    ListKind::Interface,
    ListKind::Class,
    ListKind::TypeLiteral,
    ListKind::Namespace,
    ListKind::AmbientModule,
    ListKind::Augmentation,
    ListKind::Global,
];

impl ListKind {
    fn is_module_block(self) -> bool {
        matches!(self, ListKind::Namespace | ListKind::AmbientModule | ListKind::Augmentation | ListKind::Global)
    }
}

// The parser's `CENSUS_*` bits (parser_1.rs).
const THIS: u16 = 1;
const IMPORT: u16 = 2;
const INFER: u16 = 4;
const FLOW: u16 = 8;
const DIAG: u16 = 16;
const JSDOC: u16 = 32;
const REWIND: u16 = 64;
const OTHER: u16 = 128;
const SIZE: u16 = 256;
const MODREF: u16 = 512;
const EXPORT_LOCAL: u16 = 1024;
const BIT_NAMES: [&str; 11] =
    ["this", "import", "infer", "flow", "diagnostic", "eager JSDoc", "not rewindable", "other", "size", "module refs", "export {x}"];

struct ListRec {
    kind: ListKind,
    file: u32,
    parent: Option<u32>,
    owner: Option<u32>,
    bytes: u64,
    disq: u16,
    /// In a global library file, or its owner merges into the global scope more than once: forced before checking
    /// (fileregions.rs `force_shared_lists`).
    pre: bool,
}

struct FileRec {
    name: String,
    skip: bool,
    global_lib: bool,
    parse: u64,
    bind: u64,
}

struct State {
    files: Vec<FileRec>,
    lists: Vec<ListRec>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn addr<T>(p: P<T>) -> usize {
    census::addr_of(p.get())
}

pub(crate) fn register(program: &Program) {
    if !census::enabled() {
        return;
    }
    program.bind_source_files();
    let records = census::take_records();
    let parse_lists: FxHashMap<usize, (u64, u16)> = records.lists.into_iter().map(|(l, b, d)| (l, (b, d))).collect();
    let bind_members: FxHashMap<usize, u64> = records.bind_members.into_iter().collect();
    let parse_files: FxHashMap<usize, u64> = records.parse_files.into_iter().collect();
    let bind_files: FxHashMap<usize, u64> = records.bind_files.into_iter().collect();

    let global_lib: FxHashSet<P<ast::SourceFile>> = crate::fileregions::global_library_files(program).into_iter().collect();
    // Symbols that merge into the global scope more than once (`force_shared_lists`).
    let mut merged: FxHashSet<usize> = FxHashSet::default();
    {
        let mut by_name: FxHashMap<&'static str, Vec<P<Symbol>>> = FxHashMap::default();
        let mut add = |table: Option<P<tsrs_ast::SymbolTable>>| {
            if let Some(table) = table {
                table.for_each(|name, symbol| by_name.entry(name).or_default().push(symbol));
            }
        };
        for &file in program.files.iter() {
            if !ast::is_external_or_common_js_module(file) {
                add(file.locals());
            }
            for &augmentation in file.module_augmentations() {
                let declaration = augmentation.parent().unwrap();
                if ast::is_global_scope_augmentation(declaration) {
                    add(declaration.symbol().and_then(|s| s.exports()));
                }
            }
        }
        #[expect(clippy::iter_over_hash_type, reason = "builds a set")]
        for symbols in by_name.values() {
            if symbols.len() > 1 {
                merged.extend(symbols.iter().map(|&s| addr(s)));
            }
        }
    }

    let mut st = State { files: Vec::new(), lists: Vec::new() };
    let mut list_map: FxHashMap<usize, u32> = FxHashMap::default();
    let mut owner_map: FxHashMap<usize, u32> = FxHashMap::default();

    for &file in program.files.iter() {
        if !file.is_declaration_file() {
            continue;
        }
        let skip = program.skip_type_checking(file, false);
        let file_idx = st.files.len() as u32;
        let is_global_lib = global_lib.contains(&file);
        st.files.push(FileRec {
            name: file.file_name().to_string(),
            skip,
            global_lib: is_global_lib,
            parse: parse_files.get(&addr(file)).copied().unwrap_or(0),
            bind: bind_files.get(&addr(file)).copied().unwrap_or(0),
        });
        if !skip {
            continue;
        }
        let mut w = Walker {
            st: &mut st,
            file: file_idx,
            external: ast::is_external_module(file),
            global_lib: is_global_lib,
            merged: &merged,
            stack: Vec::new(),
            parse_lists: &parse_lists,
            bind_members: &bind_members,
            list_map: &mut list_map,
            owner_map: &mut owner_map,
        };
        w.visit(file.as_node());
    }

    let n_lists = list_map.len();
    let n_owners = owner_map.len();
    census::install(census::Registry {
        lists: list_map,
        owners: owner_map,
        members: FxHashMap::default(),
        list_bits: (0..n_lists).map(|_| AtomicU8::new(0)).collect(),
        owner_bits: (0..n_owners).map(|_| AtomicU8::new(0)).collect(),
        member_bits: Vec::new(),
    });
    *STATE.lock().unwrap() = Some(st);
}

struct Walker<'a> {
    st: &'a mut State,
    file: u32,
    external: bool,
    global_lib: bool,
    merged: &'a FxHashSet<usize>,
    stack: Vec<u32>,
    parse_lists: &'a FxHashMap<usize, (u64, u16)>,
    bind_members: &'a FxHashMap<usize, u64>,
    list_map: &'a mut FxHashMap<usize, u32>,
    owner_map: &'a mut FxHashMap<usize, u32>,
}

impl Walker<'_> {
    fn visit(&mut self, node: P<Node>) {
        let candidate: Option<(ListKind, P<NodeList>, Option<P<Symbol>>)> = match node.kind() {
            Kind::InterfaceDeclaration => node.member_list().map(|l| (ListKind::Interface, l, node.symbol())),
            Kind::ClassDeclaration | Kind::ClassExpression => node.member_list().map(|l| (ListKind::Class, l, node.symbol())),
            Kind::TypeLiteral => node.member_list().map(|l| (ListKind::TypeLiteral, l, node.symbol())),
            Kind::ModuleBlock => {
                let decl = node.parent().unwrap();
                let md = decl.as_module_declaration();
                let kind = if md.keyword == Kind::GlobalKeyword {
                    ListKind::Global
                } else if md.name.kind() == Kind::StringLiteral {
                    if self.external || !self.stack.is_empty() {
                        ListKind::Augmentation
                    } else {
                        ListKind::AmbientModule
                    }
                } else {
                    ListKind::Namespace
                };
                Some((kind, node.as_module_block().statements, decl.symbol()))
            }
            _ => None,
        };
        let pushed = match candidate {
            Some((kind, list, owner)) if self.parse_lists.contains_key(&addr(list)) && !list.nodes().is_empty() => {
                let idx = self.st.lists.len() as u32;
                self.list_map.insert(addr(list), idx);
                let pre = self.global_lib || owner.is_some_and(|s| self.merged.contains(&addr(s)));
                let owner = owner.map(|s| {
                    let next = self.owner_map.len() as u32;
                    *self.owner_map.entry(addr(s)).or_insert(next)
                });
                let bind: u64 = list.nodes().iter().map(|&m| self.bind_members.get(&addr(m)).copied().unwrap_or(0)).sum();
                let (parse, disq) = self.parse_lists[&addr(list)];
                self.st.lists.push(ListRec { kind, file: self.file, parent: self.stack.last().copied(), owner, bytes: parse + bind, disq, pre });
                self.stack.push(idx);
                true
            }
            _ => false,
        };
        node.for_each_child(&mut |child| {
            self.visit(child);
            false
        });
        if pushed {
            self.stack.pop();
        }
    }
}

fn mb(b: u64) -> String {
    format!("{:.1}", b as f64 / (1024.0 * 1024.0))
}

/// One design: which lists are lazy (`eligible`), whether forcing a list of a kind parses its nested lists lazily
/// again (`nested_lazy`), and whether the global libraries' lists are forced before checking (`pre`).
struct Scenario {
    label: &'static str,
    eligible: fn(&ListRec) -> bool,
    nested_lazy: fn(ListKind) -> bool,
    pre: fn(&ListRec) -> bool,
}

struct Outcome {
    saved: u64,
    saved_lists: u64,
    lazy_lists: u64,
    forced_bytes: u64,
    per_file: Vec<u64>,
    per_kind: FxHashMap<ListKind, u64>,
}

fn simulate(st: &State, asked: &[bool], s: &Scenario) -> Outcome {
    // Child mode per list: 0 its nested lists may be lazy, 1 they are parsed eagerly (inside a forced list whose
    // nested lists are eager), 2 they are never parsed (inside a list nobody asked for).
    let mut child_mode = vec![0u8; st.lists.len()];
    let mut out = Outcome { saved: 0, saved_lists: 0, lazy_lists: 0, forced_bytes: 0, per_file: vec![0; st.files.len()], per_kind: FxHashMap::default() };
    for (i, l) in st.lists.iter().enumerate() {
        let mode = l.parent.map_or(0, |p| child_mode[p as usize]);
        if mode == 2 {
            child_mode[i] = 2;
            continue;
        }
        let lazy = mode == 0 && (s.eligible)(l);
        if !lazy {
            child_mode[i] = mode;
            continue;
        }
        out.lazy_lists += 1;
        if asked[i] || (s.pre)(l) {
            out.forced_bytes += l.bytes;
            child_mode[i] = if (s.nested_lazy)(l.kind) { 0 } else { 1 };
        } else {
            out.saved += l.bytes;
            out.saved_lists += 1;
            out.per_file[l.file as usize] += l.bytes;
            *out.per_kind.entry(l.kind).or_default() += l.bytes;
            child_mode[i] = 2;
        }
    }
    out
}

fn member_kind(k: ListKind) -> bool {
    matches!(k, ListKind::Interface | ListKind::Class | ListKind::TypeLiteral)
}

fn module_ok(l: &ListRec) -> bool {
    // `infer` in a module block declares into a conditional type inside it.
    l.kind.is_module_block() && l.disq & !INFER == 0
}

/// The census report (None unless the census ran), with `TSRS_LAZY_DTS_CENSUS_TSV=<file>` also a per-file table.
pub fn report() -> Option<String> {
    if !census::enabled() {
        return None;
    }
    census::stop();
    let guard = STATE.lock().unwrap();
    let st = guard.as_ref()?;
    let reg = census::registry()?;
    let lists = &st.lists;

    // Whether anything asked for each list: its nodes, or the owner table the list fills.
    let asked: Vec<bool> = lists
        .iter()
        .enumerate()
        .map(|(i, l)| {
            // Relaxed (this and the loads below): the checker threads that set the bits are joined.
            let mut bits = reg.list_bits[i].load(Ordering::Relaxed);
            if let Some(o) = l.owner {
                let ob = reg.owner_bits[o as usize].load(Ordering::Relaxed);
                let (members, exports) = (ob & 0xf, ob >> 4);
                bits |= match l.kind {
                    ListKind::Interface | ListKind::TypeLiteral => members,
                    ListKind::Class => members | exports,
                    _ => exports,
                };
            }
            bits != 0
        })
        .collect();

    let skip_files: Vec<&FileRec> = st.files.iter().filter(|f| f.skip).collect();
    let dts_parse: u64 = skip_files.iter().map(|f| f.parse).sum();
    let dts_bind: u64 = skip_files.iter().map(|f| f.bind).sum();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "lazy-dts census: {} unchecked declaration files ({} global libraries, {} checked), parse {} MB + bind {} MB; {} non-empty lists, {} asked for",
        skip_files.len(),
        skip_files.iter().filter(|f| f.global_lib).count(),
        st.files.len() - skip_files.len(),
        mb(dts_parse),
        mb(dts_bind),
        lists.len(),
        asked.iter().filter(|&&a| a).count()
    );
    let _ = writeln!(out, "  kind            lists   asked   outermost MB   pre-forced MB");
    for k in KINDS {
        let (mut n, mut a, mut top, mut pre) = (0u64, 0u64, 0u64, 0u64);
        for (i, l) in lists.iter().enumerate().filter(|(_, l)| l.kind == k) {
            n += 1;
            a += asked[i] as u64;
            if l.parent.is_none() {
                top += l.bytes;
                if l.pre {
                    pre += l.bytes;
                }
            }
        }
        let _ = writeln!(out, "  {:<14} {:>7} {:>7} {:>14} {:>15}", format!("{k:?}"), n, a, mb(top), mb(pre));
    }
    // Disqualifiers of outermost-level lists (no enclosing list of a member kind), never asked for, by bit.
    let mut by_bit = [[0u64; 2]; 11]; // [member kinds, module blocks]
    for (i, l) in lists.iter().enumerate() {
        if asked[i] || l.pre {
            continue;
        }
        let mut p = l.parent;
        let mut outer = true;
        while let Some(q) = p {
            if lists[q as usize].kind == l.kind || (member_kind(l.kind) && member_kind(lists[q as usize].kind)) {
                outer = false;
                break;
            }
            p = lists[q as usize].parent;
        }
        if !outer {
            continue;
        }
        for (b, slot) in by_bit.iter_mut().enumerate() {
            if l.disq & (1 << b) != 0 {
                slot[l.kind.is_module_block() as usize] += l.bytes;
            }
        }
    }
    let _ = writeln!(
        out,
        "  never asked for, not pre-forced, outermost lists with each disqualifier (MB, overlapping; member lists / module blocks): {}",
        BIT_NAMES.iter().zip(by_bit).map(|(n, [a, b])| format!("{n} {}/{}", mb(a), mb(b))).collect::<Vec<_>>().join(", ")
    );

    let rule = |l: &ListRec| l.pre;
    let rule_but_modules = |l: &ListRec| l.pre && !l.kind.is_module_block();
    let never = |_: ListKind| false;
    let modules_nested = |k: ListKind| k.is_module_block();
    let all_nested = |_: ListKind| true;
    let scenarios = [
        Scenario { label: "S0  #202 (member lists, no disqualifier)", eligible: |l| member_kind(l.kind) && l.disq == 0, nested_lazy: never, pre: rule },
        Scenario {
            label: "Sb  + member lists whose only disqualifier is import types",
            eligible: |l| member_kind(l.kind) && l.disq & !IMPORT == 0,
            nested_lazy: never,
            pre: rule,
        },
        Scenario {
            label: "Sc  + member lists whose only disqualifier is eager JSDoc",
            eligible: |l| member_kind(l.kind) && l.disq & !JSDOC == 0,
            nested_lazy: never,
            pre: rule,
        },
        Scenario {
            label: "Sa  + namespace bodies (no disqualifier)",
            eligible: |l| (member_kind(l.kind) && l.disq == 0) || (l.kind == ListKind::Namespace && module_ok(l)),
            nested_lazy: modules_nested,
            pre: rule,
        },
        Scenario {
            label: "Sa' + all module blocks (no disqualifier)",
            eligible: |l| (member_kind(l.kind) && l.disq == 0) || module_ok(l),
            nested_lazy: modules_nested,
            pre: rule,
        },
        Scenario {
            label: "Sa\" + all module blocks, global libraries' module blocks not pre-forced",
            eligible: |l| (member_kind(l.kind) && l.disq == 0) || module_ok(l),
            nested_lazy: modules_nested,
            pre: rule_but_modules,
        },
        Scenario {
            label: "Sa* + all module blocks, no size limit for them",
            eligible: |l| (member_kind(l.kind) && l.disq == 0) || (l.kind.is_module_block() && l.disq & !(INFER | SIZE) == 0),
            nested_lazy: modules_nested,
            pre: rule,
        },
        Scenario {
            label: "Sabc* a* + b + c",
            eligible: |l| {
                (member_kind(l.kind) && l.disq & !(IMPORT | JSDOC) == 0)
                    || (l.kind.is_module_block() && l.disq & !(INFER | IMPORT | JSDOC | SIZE) == 0)
            },
            nested_lazy: modules_nested,
            pre: rule,
        },
        Scenario {
            label: "Sabc  a' + b + c",
            eligible: |l| {
                (member_kind(l.kind) && l.disq & !(IMPORT | JSDOC) == 0)
                    || (l.kind.is_module_block() && l.disq & !(INFER | IMPORT | JSDOC) == 0)
            },
            nested_lazy: modules_nested,
            pre: rule,
        },
        Scenario {
            label: "Sd  S0, forcing a member list keeps its nested lists lazy",
            eligible: |l| member_kind(l.kind) && l.disq == 0,
            nested_lazy: all_nested,
            pre: rule,
        },
        Scenario {
            label: "Smax every list of every kind, any disqualifier, nested lazy",
            eligible: |_| true,
            nested_lazy: all_nested,
            pre: rule,
        },
        Scenario { label: "Smax' Smax without pre-forcing", eligible: |_| true, nested_lazy: all_nested, pre: |_| false },
    ];
    let outcomes: Vec<Outcome> = scenarios.iter().map(|s| simulate(st, &asked, s)).collect();
    let base = &outcomes[0];
    // Net of the lazy records (96 bytes each, notes/mem-lazy-dts-members.md section 6).
    let net = |o: &Outcome| o.saved as i64 - 96 * o.lazy_lists as i64;
    let _ = writeln!(out, "  scenario                                                         saved MB  delta MB  net delta MB  lazy lists  saved lists  forced MB");
    for (s, o) in scenarios.iter().zip(&outcomes) {
        let _ = writeln!(
            out,
            "  {:<64} {:>8} {:>9} {:>13} {:>11} {:>12} {:>10}   [{}]",
            s.label,
            mb(o.saved),
            mb(o.saved.saturating_sub(base.saved)),
            format!("{:.1}", (net(o) - net(base)) as f64 / (1024.0 * 1024.0)),
            o.lazy_lists,
            o.saved_lists,
            mb(o.forced_bytes),
            KINDS.iter().filter_map(|k| o.per_kind.get(k).map(|b| format!("{k:?} {}", mb(*b)))).collect::<Vec<_>>().join(", ")
        );
    }
    // The files that gain most from a* + b + c.
    let abc = &outcomes[7];
    let mut gain: Vec<(u64, usize)> =
        (0..st.files.len()).map(|f| (abc.per_file[f].saturating_sub(base.per_file[f]), f)).filter(|&(g, _)| g > 0).collect();
    gain.sort_unstable_by(|a, b| b.cmp(a));
    let _ = writeln!(out, "  top files, Sabc* - S0:");
    for &(g, f) in gain.iter().take(12) {
        let _ = writeln!(out, "    {:>7} MB  {}", mb(g), st.files[f].name);
    }

    if let Some(path) = std::env::var_os("TSRS_LAZY_DTS_CENSUS_TSV") {
        let mut tsv = String::from("file\tskip\tglobal_lib\tparse\tbind");
        for s in &scenarios {
            let _ = write!(tsv, "\t{}", s.label.split_whitespace().next().unwrap());
        }
        tsv.push('\n');
        for (f, file) in st.files.iter().enumerate() {
            let _ = write!(tsv, "{}\t{}\t{}\t{}\t{}", file.name, file.skip, file.global_lib, file.parse, file.bind);
            for o in &outcomes {
                let _ = write!(tsv, "\t{}", o.per_file[f]);
            }
            tsv.push('\n');
        }
        let _ = std::fs::write(path, tsv);
    }
    let _ = (THIS, FLOW, DIAG, REWIND, OTHER, SIZE, MODREF, EXPORT_LOCAL);
    Some(out)
}
