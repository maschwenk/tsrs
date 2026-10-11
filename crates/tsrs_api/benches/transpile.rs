//! `transpileModule` (the native API's single-file, no-check emit) over real TypeScript inputs: parse, bind and emit
//! to JavaScript. Each call builds and frees its own one-file program in a scratch region.
//!
//! `cargo bench -p tsrs_api`; under CodSpeed: `cargo codspeed build -p tsrs_api && cargo codspeed run`.

use tsrs_api::transpile::transpile;

/// A medium implementation file: the AST node classes of the npm package (about 20 KB).
const NODE_TS: &str = include_str!("../../../npm/tsrs/src/api/node/node.ts");
/// A large implementation file: the TypeScript scanner of the npm package (about 120 KB).
const SCANNER_TS: &str = include_str!("../../../npm/tsrs/src/ast/scanner.ts");

fn main() {
    divan::main();
}

fn input(name: &str) -> &'static str {
    if name == "node.ts" { NODE_TS } else { SCANNER_TS }
}

#[divan::bench(args = ["node.ts", "scanner.ts"])]
fn transpile_module(name: &str) -> usize {
    let out = transpile(divan::black_box(input(name)), None, name, false, false).expect("transpile succeeds");
    out.output_text.len()
}
