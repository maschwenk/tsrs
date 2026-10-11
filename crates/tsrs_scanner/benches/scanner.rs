//! Scanner throughput: one full token stream (trivia skipped) over real TypeScript inputs.
//!
//! `cargo bench -p tsrs_scanner`; under CodSpeed: `cargo codspeed build -p tsrs_scanner && cargo codspeed run`.

use tsrs_ast::Kind;
use tsrs_core::LanguageVariant;
use tsrs_scanner::Scanner;

/// Declaration-heavy input: the bundled ES5 lib (about 220 KB).
const LIB_ES5: &str = include_str!("../../tsrs_vfs/libs/lib.es5.d.ts");
/// Implementation-heavy input: the TypeScript scanner of the npm package (about 120 KB).
const SCANNER_TS: &str = include_str!("../../../npm/tsrs/src/ast/scanner.ts");

fn main() {
    divan::main();
}

fn scan_all(text: &'static str) -> usize {
    let mut s = Scanner::new();
    s.set_text(text);
    s.set_language_variant(LanguageVariant::Standard);
    s.set_skip_trivia(true);
    let mut tokens = 0;
    loop {
        let tok = s.scan();
        tokens += 1;
        if tok == Kind::EndOfFile {
            break;
        }
    }
    tokens
}

#[divan::bench(args = ["lib.es5.d.ts", "scanner.ts"])]
fn scan(input: &str) -> usize {
    let text = if input == "lib.es5.d.ts" { LIB_ES5 } else { SCANNER_TS };
    scan_all(divan::black_box(text))
}
