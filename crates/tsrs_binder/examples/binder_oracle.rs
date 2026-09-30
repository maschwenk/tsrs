// Rust side of the binder oracle; mirror of tools/oracle/binder/main.go.
//
//   binder_oracle dump FILE        print the binder dump of FILE
//   binder_oracle hash < filelist  print "hash path" for every file named on stdin

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{BufRead, Write as _};

use tsrs_ast::{FlowFlags, FlowList, FlowNode, Kind, Node, NodeFlags, SourceFileParseOptions, Symbol, SymbolTable};
use tsrs_core::tspath::Path;
use tsrs_core::P;

fn binder_node_flags() -> NodeFlags {
    NodeFlags::ExportContext
        | NodeFlags::ContainsThis
        | NodeFlags::HasImplicitReturn
        | NodeFlags::HasExplicitReturn
        | NodeFlags::ThisNodeOrAnySubNodesHasError
        | NodeFlags::HasAsyncFunctions
        | NodeFlags::Unreachable
}

fn printed_name(s: &str) -> String {
    s.replace(tsrs_ast::InternalSymbolNamePrefix, "@@")
}

fn escape(sb: &mut String, s: &str) {
    let s = printed_name(s);
    for &b in s.as_bytes() {
        if (0x20..0x7f).contains(&b) && b != b'\\' && b != b' ' {
            sb.push(b as char);
        } else {
            let _ = write!(sb, "\\x{:02x}", b);
        }
    }
}

fn node_ref(n: Option<P<Node>>) -> String {
    match n {
        None => "-".to_string(),
        Some(n) => format!("{}@{}", n.kind as i16, n.pos()),
    }
}

#[derive(Default)]
struct Dumper {
    sb: String,
    ids: HashMap<P<FlowNode>, usize>,
    queue: Vec<P<FlowNode>>,
}

impl Dumper {
    fn flow_ref(&mut self, f: Option<P<FlowNode>>) -> String {
        let Some(f) = f else { return "-".to_string() };
        let next = self.ids.len() + 1;
        let id = *self.ids.entry(f).or_insert_with(|| next);
        if id == next {
            self.queue.push(f);
        }
        format!("#{}", id)
    }

    fn flow_list(&mut self, mut l: Option<P<FlowList>>) -> String {
        let mut parts = Vec::new();
        while let Some(list) = l {
            parts.push(self.flow_ref(Some(list.flow)));
            l = list.next.get();
        }
        format!("[{}]", parts.join(","))
    }

    fn table(&mut self, tag: &str, t: Option<P<SymbolTable>>) {
        let Some(t) = t else { return };
        let mut entries = t.entries();
        entries.sort_by(|a, b| printed_name(a.0).cmp(&printed_name(b.0)));
        let _ = write!(self.sb, " {}{{", tag);
        for (i, (name, symbol)) in entries.iter().enumerate() {
            if i > 0 {
                self.sb.push(' ');
            }
            escape(&mut self.sb, name);
            let _ = write!(self.sb, ":{:x}", symbol.flags.get().bits());
        }
        self.sb.push('}');
    }

    fn symbol(&mut self, tag: &str, s: P<Symbol>) {
        let _ = write!(self.sb, " {}(", tag);
        escape(&mut self.sb, s.name.get());
        let _ = write!(self.sb, " {:x} {} {}", s.flags.get().bits(), s.declarations().len(), node_ref(s.value_declaration.get()));
        if let Some(parent) = s.parent.get() {
            self.sb.push_str(" p=");
            escape(&mut self.sb, parent.name.get());
        }
        if let Some(export_symbol) = s.export_symbol.get() {
            let _ = write!(self.sb, " x={:x}", export_symbol.flags.get().bits());
        }
        self.table("X", s.exports.get());
        self.table("M", s.members.get());
        self.sb.push(')');
    }

    fn visit(&mut self, n: P<Node>) -> bool {
        let old = std::mem::take(&mut self.sb);
        let f = n.flags() & binder_node_flags();
        if !f.is_empty() {
            let _ = write!(self.sb, " F{:x}", f.bits());
        }
        if let Some(symbol) = n.declaration_data().and_then(|d| d.symbol.get()) {
            self.symbol("S", symbol);
        }
        if let Some(local_symbol) = n.exportable_data().and_then(|d| d.local_symbol.get()) {
            self.symbol("LS", local_symbol);
        }
        if let Some(data) = n.locals_container_data() {
            self.table("L", data.locals.get());
            if let Some(next) = data.next_container.get() {
                let _ = write!(self.sb, " next={}", node_ref(Some(next)));
            }
        }
        if let Some(flow) = n.flow_node_data().and_then(|d| d.flow_node.get()) {
            let r = self.flow_ref(Some(flow));
            let _ = write!(self.sb, " f={}", r);
        }
        if let Some(end) = n.body_data().and_then(|d| d.end_flow_node.get()) {
            let r = self.flow_ref(Some(end));
            let _ = write!(self.sb, " e={}", r);
        }
        let ret = match n.kind {
            Kind::Constructor => n.as_constructor_declaration().return_flow_node(),
            Kind::FunctionDeclaration => n.as_function_declaration().return_flow_node(),
            Kind::FunctionExpression => n.as_function_expression().return_flow_node(),
            Kind::ClassStaticBlockDeclaration => n.as_class_static_block_declaration().return_flow_node(),
            Kind::CaseClause | Kind::DefaultClause => {
                if let Some(t) = n.as_case_or_default_clause().fallthrough_flow_node() {
                    let r = self.flow_ref(Some(t));
                    let _ = write!(self.sb, " t={}", r);
                }
                None
            }
            _ => None,
        };
        if ret.is_some() {
            let r = self.flow_ref(ret);
            let _ = write!(self.sb, " r={}", r);
        }
        let body = std::mem::replace(&mut self.sb, old);
        if !body.is_empty() {
            let _ = writeln!(self.sb, "N {} {} {}{}", n.kind as i16, n.pos(), n.end(), body);
        }
        n.for_each_child(&mut |child| self.visit(child))
    }

    fn flow_node(&mut self, id: usize, f: P<FlowNode>) {
        let _ = write!(self.sb, "F #{} {:x} ", id, f.flags.get().bits());
        if f.flags.get().intersects(FlowFlags::SwitchClause) {
            let data = f.node.get().unwrap().as_flow_switch_clause_data();
            let _ = write!(self.sb, "SW({},{},{})", node_ref(Some(data.switch_statement)), data.clause_start, data.clause_end);
        } else if f.flags.get().intersects(FlowFlags::ReduceLabel) {
            let data = f.node.get().unwrap().as_flow_reduce_label_data();
            let target = self.flow_ref(Some(data.target));
            let list = self.flow_list(data.antecedents);
            let _ = write!(self.sb, "RL({},{})", target, list);
        } else {
            self.sb.push_str(&node_ref(f.node.get()));
        }
        let antecedent = self.flow_ref(f.antecedent.get());
        let antecedents = self.flow_list(f.antecedents.get());
        let _ = writeln!(self.sb, " {} {}", antecedent, antecedents);
    }
}

fn file_name_directive(line: &str) -> Option<String> {
    let line = line.trim_end_matches(['\r', '\n']).trim_start_matches([' ', '\t']);
    let line = line.strip_prefix("//")?.trim_start_matches([' ', '\t']);
    if line.len() < 9 || !line.is_char_boundary(9) || line[..9].to_ascii_lowercase() != "@filename" {
        return None;
    }
    let line = line[9..].trim_start_matches([' ', '\t']);
    let line = line.strip_prefix(':')?;
    Some(line.trim_matches([' ', '\t']).to_string())
}

// Invalid UTF-8 input bytes become U+FFFD here (Go keeps them), so a handful of binary-ish test files differ.
fn dump(path: &str) -> Result<String, String> {
    let text = std::fs::read(path).map_err(|e| format!("{}: {}", path, e))?;
    let text = String::from_utf8_lossy(&text).into_owned();
    let mut out = String::new();
    let mut name = std::path::Path::new(path).file_name().unwrap().to_string_lossy().into_owned();
    let mut unit = String::new();
    for line in text.split_inclusive('\n') {
        if let Some(next) = file_name_directive(line) {
            dump_unit(&mut out, &name, &unit);
            name = next;
            unit.clear();
            continue;
        }
        unit.push_str(line);
    }
    dump_unit(&mut out, &name, &unit);
    Ok(out)
}

fn dump_unit(out: &mut String, name: &str, text: &str) {
    let file_name = tsrs_core::tspath::normalize_path(&format!("/oracle/{}", name.trim_start_matches('/')));
    out.push_str("U ");
    escape(out, &file_name);
    out.push('\n');
    let script_kind = tsrs_core::get_script_kind_from_file_name(&file_name);
    if script_kind == tsrs_core::ScriptKind::Unknown {
        out.push_str("skipped\n");
        return;
    }
    let opts = SourceFileParseOptions { file_name: file_name.clone(), path: Path::new(file_name.clone()), external_module_indicator_options: Default::default() };
    let file = tsrs_parser::parse_source_file(opts, text, script_kind);
    tsrs_binder::bind_source_file(file);
    let mut d = Dumper::default();
    d.visit(file.as_node());
    let mut i = 0;
    while i < d.queue.len() {
        let f = d.queue[i];
        d.flow_node(i + 1, f);
        i += 1;
    }
    for diag in file.bind_diagnostics() {
        let _ = write!(d.sb, "B {} {} {} ", diag.code(), diag.pos(), diag.len());
        escape(&mut d.sb, diag.message_text());
        d.sb.push_str(" R");
        for r in diag.related_information() {
            let _ = write!(d.sb, " {}:{}:{}", r.code(), r.pos(), r.len());
        }
        d.sb.push('\n');
    }
    let _ = writeln!(d.sb, "C {} {}", file.symbol_count.get(), file.pattern_ambient_modules.get().len());
    out.push_str(&d.sb);
}

fn fnv64a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args[1].as_str() {
        "dump" => match dump(&args[2]) {
            Ok(out) => print!("{}", out),
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        },
        "hash" => {
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let mut w = std::io::BufWriter::new(stdout.lock());
            for line in stdin.lock().lines() {
                let path = line.unwrap();
                let result = std::panic::catch_unwind(|| dump(&path));
                match result {
                    Ok(Ok(out)) => {
                        let _ = writeln!(w, "{:016x} {}", fnv64a(out.as_bytes()), path);
                    }
                    Ok(Err(e)) => eprintln!("{}", e),
                    Err(_) => {
                        let _ = writeln!(w, "PANIC {}", path);
                    }
                }
            }
        }
        _ => {}
    }
}
