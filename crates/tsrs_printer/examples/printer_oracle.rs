// Rust side of the printer oracle; mirror of tools/oracle/printer/main.go (see there for the format).
//
//   printer_oracle print MODE FILE [FLAGS]   print FILE with the printer
//   printer_oracle hash MODE < list          print "hash path" for every "path[\tFLAGS]" line on stdin

use std::io::{BufRead, Write as _};

use tsrs_ast::{ExternalModuleIndicatorOptions, SourceFileParseOptions};
use tsrs_core::tspath::Path;
use tsrs_ast::{Node, SourceFile};
use tsrs_core::P;
use tsrs_printer::{get_single_line_string_writer, new_emit_context, new_printer, new_text_writer, EmitContext, EmitFlags, EmitTextWriter, PrintHandlers, PrinterOptions};

fn parse_flags(flags: &str) -> ExternalModuleIndicatorOptions {
    ExternalModuleIndicatorOptions { jsx: flags.contains('j'), force: flags.contains('f') }
}

fn read(path: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(tsrs_vfs::internal::decode_bytes(bytes))
}

fn options(mode: &str) -> PrinterOptions {
    match mode {
        "default" => PrinterOptions::default(),
        "nocomments" => PrinterOptions { remove_comments: true, ..Default::default() },
        "omitsemi" => PrinterOptions { remove_comments: true, omit_trailing_semicolon: true, never_ascii_escape: true, ..Default::default() },
        "preserve" => PrinterOptions {
            new_line: tsrs_core::NewLineKind::CRLF,
            never_ascii_escape: true,
            preserve_source_newlines: true,
            terminate_unterminated_literals: true,
            target: tsrs_core::ScriptTarget::ES2021,
            ..Default::default()
        },
        other => panic!("unknown mode {}", other),
    }
}

/// Ok(text) or Err("SKIP"/"PANIC"/"UNREADABLE").
fn print_file(mode: &str, path: &str, flags: &str) -> Result<String, &'static str> {
    let Some(text) = read(path) else {
        return Err("UNREADABLE");
    };
    let opts = SourceFileParseOptions { file_name: path.to_string(), path: Path::new(path.to_string()), external_module_indicator_options: parse_flags(flags) };
    let file = tsrs_parser::parse_source_file(opts, &text, tsrs_core::ensure_script_kind_from_file_name(path));
    if !file.diagnostics().is_empty() {
        return Err("SKIP");
    }
    let mode = mode.to_string();
    Ok({
        if mode.starts_with("synth") {
            return Ok(print_synthesized(&mode, file));
        }
        let mut p = new_printer(options(&mode), PrintHandlers::default(), None);
        p.emit_source_file(file)
    })
}

fn set_flags_recursive(ec: P<EmitContext>, node: P<Node>, flags: EmitFlags) {
    ec.add_emit_flags(node, flags);
    node.for_each_child(&mut |child| {
        set_flags_recursive(ec, child, flags);
        false
    });
}

fn print_synthesized(mode: &str, file: P<SourceFile>) -> String {
    let ec = new_emit_context();
    let mut p = new_printer(PrinterOptions { remove_comments: true, ..Default::default() }, PrintHandlers::default(), Some(ec));
    let mut sb = String::new();
    for &stmt in file.statements.nodes() {
        let clone = ec.factory.deep_clone_node(Some(stmt)).unwrap();
        let mut w: Box<dyn EmitTextWriter> = match mode {
            "synth" => new_text_writer("\n", 0),
            "synthflags" => {
                set_flags_recursive(ec, clone, EmitFlags::SingleLine | EmitFlags::NoAsciiEscaping);
                get_single_line_string_writer().0
            }
            "synthmulti" => {
                set_flags_recursive(ec, clone, EmitFlags::MultiLine | EmitFlags::StartOnNewLine | EmitFlags::Indented);
                new_text_writer("\n", 0)
            }
            other => panic!("unknown mode {}", other),
        };
        p.write(clone, None, &mut w, None);
        sb.push_str(&w.string());
        sb.push_str("\n---\n");
    }
    sb
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
        "print" => {
            let flags = args.get(4).map(|s| s.as_str()).unwrap_or("-");
            match print_file(&args[2], &args[3], flags) {
                Ok(text) => std::io::stdout().write_all(text.as_bytes()).unwrap(),
                Err(status) => eprintln!("{}", status),
            }
        }
        "hash" => {
            let mode = args[2].clone();
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let mut w = std::io::BufWriter::new(stdout.lock());
            for line in stdin.lock().lines() {
                let line = line.unwrap();
                let (path, flags) = split_line(&line);
                match print_file(&mode, path, flags) {
                    Ok(text) => {
                        let _ = writeln!(w, "{:016x} {}", fnv64a(text.as_bytes()), path);
                    }
                    Err("UNREADABLE") => {}
                    Err(status) => {
                        let _ = writeln!(w, "{} {}", status, path);
                    }
                }
                let _ = w.flush();
            }
        }
        other => panic!("unknown command {}", other),
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        if std::env::var("TSRS_ORACLE_VERBOSE").is_ok() {
            eprintln!("{}", info);
        }
    }));
    let child = std::thread::Builder::new().stack_size(4096 << 20).spawn(run).unwrap();
    if child.join().is_err() {
        std::process::exit(101);
    }
}
