// Exact comparison of format_document against the Go formatter: tools/oracle/format prints the Go edits for each file
// of a list; this test recomputes them and compares. Opt-in (needs the Go oracle output):
//   TSRS_FORMAT_ORACLE=<oracle output> TSRS_FORMAT_ORACLE_PRESET=<default|api|alt|insert|indent> \
//     cargo test -p tsrs_ls --lib format::oracle_test
use tsrs_core::{ScriptKind, Tristate};

use super::api_test::settings;
use super::*;
use crate::astnav::parse_for_test;
use crate::lsutil;

fn serialize(edits: &[tsrs_core::TextChange]) -> String {
    let mut s = String::new();
    for e in edits {
        s.push_str(&format!("{},{},{};", e.pos(), e.end(), tsrs_core::json::marshal_string(&e.new_text)));
    }
    s
}

#[test]
fn test_format_document_matches_go_oracle() {
    // Deeply nested corpus files need the stack size the server's worker threads get.
    std::thread::Builder::new().stack_size(512 << 20).spawn(run_oracle).unwrap().join().unwrap();
}

fn run_oracle() {
    let Ok(oracle) = std::env::var("TSRS_FORMAT_ORACLE") else {
        eprintln!("skipping: TSRS_FORMAT_ORACLE not set");
        return;
    };
    let options = match std::env::var("TSRS_FORMAT_ORACLE_PRESET").as_deref() {
        Ok("api") => settings(4, 4, 4, Tristate::True, |s| s.insert_space_before_type_annotation = Tristate::True),
        Ok(preset @ ("alt" | "insert")) => {
            let mut s = lsutil::get_default_format_code_settings();
            s.tab_size = 2;
            s.indent_size = 2;
            s.base_indent_size = 1;
            s.convert_tabs_to_spaces = Tristate::False;
            s.insert_space_after_comma_delimiter = Tristate::False;
            s.insert_space_after_semicolon_in_for_statements = Tristate::False;
            s.insert_space_before_and_after_binary_operators = Tristate::False;
            s.insert_space_after_constructor = Tristate::True;
            s.insert_space_after_keywords_in_control_flow_statements = Tristate::False;
            s.insert_space_after_function_keyword_for_anonymous_functions = Tristate::True;
            s.insert_space_after_opening_and_before_closing_nonempty_parenthesis = Tristate::True;
            s.insert_space_after_opening_and_before_closing_nonempty_brackets = Tristate::True;
            s.insert_space_after_opening_and_before_closing_nonempty_braces = Tristate::False;
            s.insert_space_after_opening_and_before_closing_empty_braces = Tristate::True;
            s.insert_space_after_opening_and_before_closing_template_string_braces = Tristate::True;
            s.insert_space_after_opening_and_before_closing_jsx_expression_braces = Tristate::True;
            s.insert_space_after_type_assertion = Tristate::True;
            s.insert_space_before_function_parenthesis = Tristate::True;
            s.place_open_brace_on_new_line_for_functions = Tristate::True;
            s.place_open_brace_on_new_line_for_control_blocks = Tristate::True;
            s.insert_space_before_type_annotation = Tristate::True;
            s.indent_multi_line_object_literal_beginning_on_blank_line = Tristate::True;
            s.indent_switch_case = Tristate::False;
            s.trim_trailing_whitespace = Tristate::False;
            s.semicolons = lsutil::SemicolonPreference::Remove;
            if preset == "insert" {
                s.semicolons = lsutil::SemicolonPreference::Insert;
                s.new_line_character = "\r\n".to_string();
            }
            s
        }
        _ => lsutil::get_default_format_code_settings(),
    };
    let indent_mode = std::env::var("TSRS_FORMAT_ORACLE_PRESET").as_deref() == Ok("indent");
    let ctx = with_format_code_settings(&FormatContext::default(), options.clone(), "\n");
    let expected = std::fs::read_to_string(oracle).unwrap();
    let mut total = 0;
    let mut failures: Vec<String> = Vec::new();
    for line in expected.lines() {
        let (path, want) = line.split_once('\t').unwrap();
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        total += 1;
        let (name, kind) = if path.ends_with(".tsx") { ("/file.tsx", ScriptKind::TSX) } else { ("/file.ts", ScriptKind::TS) };
        let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let file = parse_for_test(name, &text, kind);
            if indent_mode {
                let mut s = String::new();
                for (i, &ls) in file.ecma_line_map().iter().enumerate() {
                    s.push_str(&format!("{},", get_indentation(ls as i32, file, &options, i % 2 == 0)));
                }
                return s;
            }
            serialize(&format_document(&ctx, file))
        }))
        .unwrap_or_else(|_| "PANIC".to_string());
        if got != want {
            failures.push(path.to_string());
            if failures.len() <= 5 {
                let dir = crate::astnav::repo_root().join("target/scratch/lsfound/oracle/diff");
                std::fs::create_dir_all(&dir).unwrap();
                let base = std::path::Path::new(path).file_name().unwrap().to_string_lossy().to_string();
                std::fs::write(dir.join(format!("{base}.want")), want.replace(';', ";\n")).unwrap();
                std::fs::write(dir.join(format!("{base}.got")), got.replace(';', ";\n")).unwrap();
            }
        }
    }
    eprintln!("format oracle: {}/{} files match", total - failures.len(), total);
    assert!(failures.is_empty(), "{} mismatches, first: {:?}", failures.len(), &failures[..failures.len().min(20)]);
}
