//! Parser throughput over real TypeScript inputs.
//!
//! Each iteration parses inside its own arena region, freed afterwards, so repeated iterations do not grow the
//! process (outside a region the AST would live in the never-freed thread arena).
//!
//! `cargo bench -p tsrs_parser`; under CodSpeed: `cargo codspeed build -p tsrs_parser && cargo codspeed run`.

use tsrs_ast::SourceFileParseOptions;
use tsrs_core::arena::Region;
use tsrs_core::tspath::Path;
use tsrs_core::ScriptKind;

/// Declaration-heavy input: the bundled ES5 lib (about 220 KB).
const LIB_ES5: &str = include_str!("../../tsrs_vfs/libs/lib.es5.d.ts");
/// Implementation-heavy input: the TypeScript scanner of the npm package (about 120 KB).
const SCANNER_TS: &str = include_str!("../../../npm/tsrs/src/ast/scanner.ts");

fn main() {
    divan::main();
}

fn input(name: &str) -> (&'static str, &'static str) {
    match name {
        "lib.es5.d.ts" => ("/lib.es5.d.ts", LIB_ES5),
        _ => ("/scanner.ts", SCANNER_TS),
    }
}

#[divan::bench(args = ["lib.es5.d.ts", "scanner.ts"])]
fn parse(bencher: divan::Bencher, name: &str) {
    let (file_name, text) = input(name);
    bencher.bench_local(|| {
        let region = Region::new(1 << 20);
        let statements = {
            let _scope = region.enter();
            let opts = SourceFileParseOptions { file_name: file_name.to_string(), path: Path::new(file_name.to_string()), external_module_indicator_options: Default::default() };
            let file = tsrs_parser::parse_source_file_owned(opts, divan::black_box(text).to_string(), ScriptKind::TS);
            file.statements().nodes().len()
        };
        drop(region);
        statements
    });
}
