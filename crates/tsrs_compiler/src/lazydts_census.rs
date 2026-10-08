//! Lazy declaration-file member census, program side (alloc-profile builds, `TSRS_LAZY_DTS_CENSUS=1`; see
//! `tsrs_core::lazydts_census`). `register` runs before the first checker is created: it walks every declaration
//! file the program will not type-check and registers the non-empty member lists of interfaces, classes, type
//! literals and module blocks with their owner and member symbols. `report` prints, after the check, how many of
//! those lists any reader asked for and the parse and bind bytes of the ones nobody did.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

use rustc_hash::FxHashMap;
use tsrs_ast::{Kind, Node, NodeList, Symbol};
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
    Global,
}

const KINDS: [ListKind; 6] =
    [ListKind::Interface, ListKind::Class, ListKind::TypeLiteral, ListKind::Namespace, ListKind::AmbientModule, ListKind::Global];

struct ListRec {
    kind: ListKind,
    file: u32,
    parent: Option<u32>,
    owner: Option<u32>,
    parse: u64,
    disq: u8,
    bind: u64,
    members: std::ops::Range<u32>,
}

struct MemberRec {
    parse: u64,
    bind: u64,
    symbol: Option<u32>,
}

struct FileRec {
    name: String,
    skip: bool,
    parse: u64,
    bind: u64,
}

struct State {
    files: Vec<FileRec>,
    lists: Vec<ListRec>,
    members: Vec<MemberRec>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn addr<T>(p: P<T>) -> usize {
    census::addr_of(p.get())
}

pub(crate) fn register(program: &Program) {
    if !census::enabled() {
        return;
    }
    let records = census::take_records();
    let parse_lists: FxHashMap<usize, (u64, u8)> = records.lists.into_iter().map(|(l, b, d)| (l, (b, d))).collect();
    let parse_members: FxHashMap<usize, u64> = records.parse_members.into_iter().collect();
    let bind_members: FxHashMap<usize, u64> = records.bind_members.into_iter().collect();
    let parse_files: FxHashMap<usize, u64> = records.parse_files.into_iter().collect();
    let bind_files: FxHashMap<usize, u64> = records.bind_files.into_iter().collect();

    let mut st = State { files: Vec::new(), lists: Vec::new(), members: Vec::new() };
    let mut list_map: FxHashMap<usize, u32> = FxHashMap::default();
    let mut owner_map: FxHashMap<usize, u32> = FxHashMap::default();
    let mut member_map: FxHashMap<usize, u32> = FxHashMap::default();

    for &file in program.files.iter() {
        if !file.is_declaration_file() {
            continue;
        }
        let skip = program.skip_type_checking(file, false);
        let file_idx = st.files.len() as u32;
        st.files.push(FileRec {
            name: file.file_name().to_string(),
            skip,
            parse: parse_files.get(&addr(file)).copied().unwrap_or(0),
            bind: bind_files.get(&addr(file)).copied().unwrap_or(0),
        });
        if !skip {
            continue;
        }
        let mut w = Walker {
            st: &mut st,
            file: file_idx,
            stack: Vec::new(),
            parse_lists: &parse_lists,
            parse_members: &parse_members,
            bind_members: &bind_members,
            list_map: &mut list_map,
            owner_map: &mut owner_map,
            member_map: &mut member_map,
        };
        w.visit(file.as_node());
    }

    let n_lists = list_map.len();
    let n_owners = owner_map.len();
    let n_members = member_map.len();
    census::install(census::Registry {
        lists: list_map,
        owners: owner_map,
        members: member_map,
        list_bits: (0..n_lists).map(|_| AtomicU8::new(0)).collect(),
        owner_bits: (0..n_owners).map(|_| AtomicU8::new(0)).collect(),
        member_bits: (0..n_members).map(|_| AtomicU8::new(0)).collect(),
    });
    *STATE.lock().unwrap() = Some(st);
}

struct Walker<'a> {
    st: &'a mut State,
    file: u32,
    stack: Vec<u32>,
    parse_lists: &'a FxHashMap<usize, (u64, u8)>,
    parse_members: &'a FxHashMap<usize, u64>,
    bind_members: &'a FxHashMap<usize, u64>,
    list_map: &'a mut FxHashMap<usize, u32>,
    owner_map: &'a mut FxHashMap<usize, u32>,
    member_map: &'a mut FxHashMap<usize, u32>,
}

impl Walker<'_> {
    fn visit(&mut self, node: P<Node>) {
        let candidate: Option<(ListKind, P<NodeList>, Option<P<Symbol>>)> = match node.kind() {
            Kind::InterfaceDeclaration => node.member_list().map(|l| (ListKind::Interface, l, node.symbol())),
            Kind::ClassDeclaration | Kind::ClassExpression => node.member_list().map(|l| (ListKind::Class, l, node.symbol())),
            Kind::TypeLiteral => node.member_list().map(|l| (ListKind::TypeLiteral, l, node.symbol())),
            Kind::ModuleBlock => {
                let decl = node.parent().unwrap();
                let kind = if decl.as_module_declaration().keyword == Kind::GlobalKeyword {
                    ListKind::Global
                } else if decl.as_module_declaration().name.kind() == Kind::StringLiteral {
                    ListKind::AmbientModule
                } else {
                    ListKind::Namespace
                };
                Some((kind, node.as_module_block().statements, decl.symbol()))
            }
            _ => None,
        };
        let pushed = match candidate {
            Some((kind, list, owner)) if !list.nodes().is_empty() => {
                let idx = self.st.lists.len() as u32;
                self.list_map.insert(addr(list), idx);
                let owner = owner.map(|s| {
                    let next = self.owner_map.len() as u32;
                    *self.owner_map.entry(addr(s)).or_insert(next)
                });
                let start = self.st.members.len() as u32;
                let mut bind = 0;
                for &m in list.nodes() {
                    let mb = self.bind_members.get(&addr(m)).copied().unwrap_or(0);
                    bind += mb;
                    let mut sym = m.symbol();
                    if sym.is_none() && m.kind() == Kind::VariableStatement {
                        sym = m.as_variable_statement().declaration_list.as_variable_declaration_list().declarations.nodes().first().and_then(|d| d.symbol());
                    }
                    let symbol = sym.map(|s| {
                        let next = self.member_map.len() as u32;
                        let i = *self.member_map.entry(addr(s)).or_insert(next);
                        if let Some(e) = s.export_symbol() {
                            self.member_map.entry(addr(e)).or_insert(i);
                        }
                        i
                    });
                    self.st.members.push(MemberRec { parse: self.parse_members.get(&addr(m)).copied().unwrap_or(0), bind: mb, symbol });
                }
                self.st.lists.push(ListRec {
                    kind,
                    file: self.file,
                    parent: self.stack.last().copied(),
                    owner,
                    parse: self.parse_lists.get(&addr(list)).map_or(0, |e| e.0),
                    disq: self.parse_lists.get(&addr(list)).map_or(0, |e| e.1),
                    bind,
                    members: start..self.st.members.len() as u32,
                });
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

    // Phase bits under which each list was asked for (0 = never).
    let forced: Vec<u8> = lists
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
                    ListKind::Namespace | ListKind::AmbientModule | ListKind::Global => exports,
                };
            }
            bits
        })
        .collect();
    // Whether some enclosing list is never asked for (then this one is inside a saved subtree already).
    let mut inside_unforced = vec![false; lists.len()];
    for (i, l) in lists.iter().enumerate() {
        if let Some(p) = l.parent {
            inside_unforced[i] = inside_unforced[p as usize] || forced[p as usize] == 0;
        }
    }

    let skip_files: Vec<&FileRec> = st.files.iter().filter(|f| f.skip).collect();
    let dts_parse: u64 = skip_files.iter().map(|f| f.parse).sum();
    let dts_bind: u64 = skip_files.iter().map(|f| f.bind).sum();
    let checked_dts = st.files.iter().filter(|f| !f.skip).count();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "lazy-dts census: {} unchecked declaration files ({} checked), parse {} MB + bind {} MB",
        skip_files.len(),
        checked_dts,
        mb(dts_parse),
        mb(dts_bind)
    );
    let top: Vec<usize> = (0..lists.len()).filter(|&i| lists[i].parent.is_none()).collect();
    let top_parse: u64 = top.iter().map(|&i| lists[i].parse).sum();
    let top_bind: u64 = top.iter().map(|&i| lists[i].bind).sum();
    let _ = writeln!(
        out,
        "  lists: {} non-empty ({} outermost), members {}; inside outermost lists: parse {} MB + bind {} MB",
        lists.len(),
        top.len(),
        st.members.len(),
        mb(top_parse),
        mb(top_bind)
    );
    // Saved: lists never asked for whose enclosing lists were all asked for.
    let mut saved = (0u64, 0u64);
    let mut by_kind: FxHashMap<ListKind, [u64; 6]> = FxHashMap::default(); // lists, forced, top parse+bind, saved parse+bind, forced at init only
    let mut forced_phase = [0u64; 3]; // bytes (parse+bind, own) of lists first asked for in phase 1/2/4
    for (i, l) in lists.iter().enumerate() {
        let e = by_kind.entry(l.kind).or_default();
        e[0] += 1;
        if forced[i] != 0 {
            e[1] += 1;
        }
        if l.parent.is_none() {
            e[2] += l.parse + l.bind;
        }
        if forced[i] == 0 && !inside_unforced[i] {
            saved.0 += l.parse;
            saved.1 += l.bind;
            e[3] += l.parse + l.bind;
        }
        if forced[i] != 0 && !inside_unforced[i] && l.parent.is_none_or(|p| forced[p as usize] != 0) {
            let first = forced[i].trailing_zeros().min(2) as usize;
            if l.parent.is_none() {
                forced_phase[first] += l.parse + l.bind;
            }
            if forced[i] == 2 {
                e[4] += l.parse + l.bind;
            }
        }
    }
    let _ = writeln!(
        out,
        "  never asked for (outermost such lists): parse {} MB + bind {} MB = {} MB ({:.1}% of the unchecked declaration files' parse+bind)",
        mb(saved.0),
        mb(saved.1),
        mb(saved.0 + saved.1),
        100.0 * (saved.0 + saved.1) as f64 / (dts_parse + dts_bind).max(1) as f64
    );
    let _ = writeln!(
        out,
        "  outermost lists asked for first: before checkers {} MB, in new_checker {} MB, while checking {} MB",
        mb(forced_phase[0]),
        mb(forced_phase[1]),
        mb(forced_phase[2])
    );
    let _ = writeln!(out, "  kind            lists   asked   outermost MB   saved MB   asked only in new_checker MB");
    for k in KINDS {
        let e = by_kind.get(&k).copied().unwrap_or_default();
        let _ = writeln!(out, "  {:<14} {:>7} {:>7} {:>14} {:>10} {:>14}", format!("{k:?}"), e[0], e[1], mb(e[2]), mb(e[3]), mb(e[4]));
    }
    // Scenarios: module blocks of the given kinds stay eager (count as asked for); savings of the other lists.
    for (label, eager) in [
        ("module blocks eager", &[ListKind::Namespace, ListKind::AmbientModule, ListKind::Global][..]),
        ("ambient module and global blocks eager", &[ListKind::AmbientModule, ListKind::Global][..]),
    ] {
        let f2: Vec<bool> = lists.iter().enumerate().map(|(i, l)| forced[i] != 0 || eager.contains(&l.kind)).collect();
        let mut inside = vec![false; lists.len()];
        let mut sv = (0u64, 0u64);
        let mut by: FxHashMap<ListKind, u64> = FxHashMap::default();
        for (i, l) in lists.iter().enumerate() {
            if let Some(p) = l.parent {
                inside[i] = inside[p as usize] || !f2[p as usize];
            }
            if !f2[i] && !inside[i] {
                sv.0 += l.parse;
                sv.1 += l.bind;
                *by.entry(l.kind).or_default() += l.parse + l.bind;
            }
        }
        let _ = writeln!(
            out,
            "  with {label}: never asked for parse {} MB + bind {} MB = {} MB (interface {} / class {} / type literal {} / namespace {})",
            mb(sv.0),
            mb(sv.1),
            mb(sv.0 + sv.1),
            mb(by.get(&ListKind::Interface).copied().unwrap_or(0)),
            mb(by.get(&ListKind::Class).copied().unwrap_or(0)),
            mb(by.get(&ListKind::TypeLiteral).copied().unwrap_or(0)),
            mb(by.get(&ListKind::Namespace).copied().unwrap_or(0)),
        );
    }
    // The design's scope: interface, class and type literal lists without a disqualifier (`disq`, parser_1.rs
    // `census_disqualifiers`). Outermost: a forced list is parsed again with its nested lists eager; nested: a
    // forced list's nested eligible lists stay lazy.
    let eligible: Vec<bool> =
        lists.iter().map(|l| matches!(l.kind, ListKind::Interface | ListKind::Class | ListKind::TypeLiteral) && l.disq == 0).collect();
    let mut has_eligible_ancestor = vec![false; lists.len()];
    let mut unforced_eligible_ancestor = vec![false; lists.len()];
    let (mut r1, mut r2) = (0u64, 0u64);
    let mut r1_by: FxHashMap<ListKind, u64> = FxHashMap::default();
    let mut disq_bytes = [0u64; 8];
    let mut eligible_top = 0u64;
    for (i, l) in lists.iter().enumerate() {
        if let Some(p) = l.parent {
            let p = p as usize;
            has_eligible_ancestor[i] = has_eligible_ancestor[p] || eligible[p];
            unforced_eligible_ancestor[i] = unforced_eligible_ancestor[p] || (eligible[p] && forced[p] == 0);
        }
        let bytes = l.parse + l.bind;
        if matches!(l.kind, ListKind::Interface | ListKind::Class | ListKind::TypeLiteral) && !has_eligible_ancestor[i] {
            for b in 0..7 {
                if l.disq & (1 << b) != 0 {
                    disq_bytes[b] += bytes;
                }
            }
        }
        if !eligible[i] {
            continue;
        }
        if !has_eligible_ancestor[i] {
            eligible_top += bytes;
            if forced[i] == 0 {
                r1 += bytes;
                *r1_by.entry(l.kind).or_default() += bytes;
            }
        }
        if !unforced_eligible_ancestor[i] && forced[i] == 0 {
            r2 += bytes;
        }
    }
    let _ = writeln!(
        out,
        "  design scope (interface/class/type literal, no disqualifier): outermost eligible lists {} MB; never asked for: outermost {} MB (interface {} / class {} / type literal {}), nested {} MB",
        mb(eligible_top),
        mb(r1),
        mb(r1_by.get(&ListKind::Interface).copied().unwrap_or(0)),
        mb(r1_by.get(&ListKind::Class).copied().unwrap_or(0)),
        mb(r1_by.get(&ListKind::TypeLiteral).copied().unwrap_or(0)),
        mb(r2)
    );
    let _ = writeln!(
        out,
        "  disqualified (outermost-level lists, MB, overlapping): this {} / import {} / body-decorator-async {} / initializer {} / diagnostic {} / eager JSDoc {} / not rewindable {}",
        mb(disq_bytes[0]),
        mb(disq_bytes[1]),
        mb(disq_bytes[2]),
        mb(disq_bytes[3]),
        mb(disq_bytes[4]),
        mb(disq_bytes[5]),
        mb(disq_bytes[6])
    );
    // Members of asked-for lists (all ancestors asked for) whose symbol nobody looked up or read declarations of.
    let mut untouched = (0u64, 0u64, 0u64, 0u64); // members, parse, bind, members with a symbol
    let mut live_members = 0u64;
    for (i, l) in lists.iter().enumerate() {
        if forced[i] == 0 || inside_unforced[i] {
            continue;
        }
        for m in &st.members[l.members.start as usize..l.members.end as usize] {
            live_members += 1;
            let Some(s) = m.symbol else { continue };
            untouched.3 += 1;
            // Relaxed: as above.
            if reg.member_bits[s as usize].load(Ordering::Relaxed) == 0 {
                untouched.0 += 1;
                untouched.1 += m.parse;
                untouched.2 += m.bind;
            }
        }
    }
    let _ = writeln!(
        out,
        "  members of asked-for lists: {} ({} with a symbol); never looked up or read: {} members, parse {} MB + bind {} MB (nested lists included)",
        live_members,
        untouched.3,
        untouched.0,
        mb(untouched.1),
        mb(untouched.2)
    );

    if let Some(path) = std::env::var_os("TSRS_LAZY_DTS_CENSUS_TSV") {
        let mut per_file: Vec<(u64, u64, u64)> = vec![(0, 0, 0); st.files.len()];
        for (i, l) in lists.iter().enumerate() {
            let f = &mut per_file[l.file as usize];
            if l.parent.is_none() {
                f.0 += l.parse + l.bind;
            }
            if forced[i] == 0 && !inside_unforced[i] {
                f.1 += l.parse + l.bind;
            }
            if forced[i] == 2 && l.parent.is_none() {
                f.2 += l.parse + l.bind;
            }
        }
        let mut tsv = String::from("file\tskip\tparse\tbind\tin_lists\tsaved\tinit_only\n");
        for (f, (inl, sav, init)) in st.files.iter().zip(per_file) {
            let _ = writeln!(tsv, "{}\t{}\t{}\t{}\t{}\t{}\t{}", f.name, f.skip, f.parse, f.bind, inl, sav, init);
        }
        let _ = std::fs::write(path, tsv);
    }
    Some(out)
}
