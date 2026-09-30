// Rust side of the scanner token-stream oracle; mirror of tools/oracle/scanner/main.go.
//
//   scanner_oracle dump FILE        print the token streams of FILE
//   scanner_oracle hash < filelist  print "hash path" for every file named on stdin

use std::fmt::Write as _;
use std::io::{BufRead, Write as _};

use tsrs_ast::Kind;
use tsrs_core::LanguageVariant;
use tsrs_scanner::Scanner;

fn escape(sb: &mut String, s: &str) {
    for &b in s.as_bytes() {
        if (0x20..0x7f).contains(&b) && b != b'\\' {
            sb.push(b as char);
        } else {
            let _ = write!(sb, "\\x{:02x}", b);
        }
    }
}

fn is_regex_context(prev: Kind) -> bool {
    matches!(
        prev,
        Kind::Unknown
            | Kind::OpenParenToken
            | Kind::CommaToken
            | Kind::EqualsToken
            | Kind::ColonToken
            | Kind::OpenBracketToken
            | Kind::ExclamationToken
            | Kind::AmpersandAmpersandToken
            | Kind::BarBarToken
            | Kind::QuestionToken
            | Kind::OpenBraceToken
            | Kind::CloseBraceToken
            | Kind::SemicolonToken
            | Kind::ReturnKeyword
            | Kind::TypeOfKeyword
            | Kind::EqualsEqualsToken
            | Kind::EqualsEqualsEqualsToken
            | Kind::ExclamationEqualsToken
            | Kind::ExclamationEqualsEqualsToken
            | Kind::PlusToken
            | Kind::MinusToken
            | Kind::EqualsGreaterThanToken
            | Kind::CaseKeyword
            | Kind::QuestionQuestionToken
    )
}

fn flush_errors(sb: &mut String, s: &mut Scanner) {
    if !s.has_errors() {
        return;
    }
    for e in s.take_errors() {
        let _ = write!(sb, "E {} {} {}", e.message.code(), e.start, e.length);
        for a in &e.args {
            sb.push(' ');
            escape(sb, a);
        }
        sb.push('\n');
    }
}

fn pass(sb: &mut String, text: &'static str, variant: LanguageVariant, mode: i32) {
    let _ = writeln!(sb, "P {mode}");
    let mut s = Scanner::new();
    s.set_text(text);
    s.set_language_variant(variant);
    s.set_skip_trivia(mode != 2);
    if mode == 4 {
        s.set_script_target(tsrs_core::ScriptTarget::ES5);
    }
    s.set_on_error(true);
    let mut prev = Kind::Unknown;
    let mut stack: Vec<bool> = Vec::new();
    loop {
        let mut tok = s.scan();
        if mode >= 3 {
            match tok {
                Kind::SlashToken | Kind::SlashEqualsToken => {
                    if is_regex_context(prev) {
                        flush_errors(sb, &mut s);
                        tok = s.re_scan_slash_token(true);
                    }
                }
                Kind::GreaterThanToken => tok = s.re_scan_greater_than_token(),
                Kind::TemplateHead => stack.push(true),
                Kind::OpenBraceToken => stack.push(false),
                Kind::CloseBraceToken => {
                    if let Some(top) = stack.pop() {
                        if top {
                            flush_errors(sb, &mut s);
                            tok = s.re_scan_template_token(false);
                            if tok == Kind::TemplateMiddle {
                                stack.push(true);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        flush_errors(sb, &mut s);
        let _ = write!(sb, "{} {} {} {} ", tok as i16, s.token_start(), s.token_end(), s.token_flags().bits());
        escape(sb, s.token_value());
        sb.push('\n');
        if tok == Kind::EndOfFile {
            break;
        }
        if !matches!(
            tok,
            Kind::WhitespaceTrivia | Kind::NewLineTrivia | Kind::SingleLineCommentTrivia | Kind::MultiLineCommentTrivia | Kind::ConflictMarkerTrivia
        ) {
            prev = tok;
        }
    }
    for d in s.comment_directives() {
        let _ = writeln!(sb, "D {} {} {}", d.kind as i32, d.loc.pos(), d.loc.end());
    }
}

fn dump(path: &str) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8(data).map_err(|_| format!("{path}: not UTF-8"))?;
    let text: &'static str = Box::leak(text.into_boxed_str());
    let variant = if path.ends_with(".tsx") || path.ends_with(".jsx") { LanguageVariant::JSX } else { LanguageVariant::Standard };
    let mut sb = String::new();
    for mode in 1..=4 {
        pass(&mut sb, text, variant, mode);
    }
    Ok(sb)
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
            Ok(out) => {
                std::io::stdout().write_all(out.as_bytes()).unwrap();
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        "hash" => {
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let mut w = std::io::BufWriter::new(stdout.lock());
            for line in stdin.lock().lines() {
                let path = line.unwrap();
                match dump(&path) {
                    Ok(out) => {
                        writeln!(w, "{:016x} {}", fnv64a(out.as_bytes()), path).unwrap();
                    }
                    Err(e) => eprintln!("{e}"),
                }
            }
        }
        _ => panic!("unknown mode"),
    }
}
