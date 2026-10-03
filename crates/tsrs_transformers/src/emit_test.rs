// Isolated smoke tests for the jsx and legacy decorator transforms (before emit/core wires them into the emit
// pipeline): build a program, lend the file's checker to a Resolver, run the transforms and print the result.
// Inputs are .js/.jsx files so the missing type eraser does not matter.

use std::sync::Arc;

use tsrs_compiler::{new_compiler_host, new_program, Context, ProgramOptions};
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{vfstest, FS};

use crate::*;

struct ConfigHost {
    fs: Arc<dyn FS>,
}

impl ParseConfigHost for ConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        "/"
    }
}

fn emit(files: &[(&str, &str)], file: &str, transforms: &[fn(&TransformOptions) -> P<Transformer>]) -> String {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(files.iter().map(|&(k, v)| (k, v)), true));
    let host: &'static ConfigHost = Box::leak(Box::new(ConfigHost { fs: fs.clone() }));
    let (config, diagnostics) = tsoptions::get_parsed_command_line_of_config_file("/src/tsconfig.json", None, None, host, None);
    assert!(diagnostics.is_empty());
    let config = P::new(config.unwrap());
    let program = new_program(ProgramOptions::new(config, new_compiler_host("/", fs, "", None, None)));
    let ctx = Context::default();
    let source_file = program.get_source_file(file).unwrap();
    let mut checker = program.get_type_checker_for_file(&ctx, source_file);
    let slot = P::new(tsrs_checker::CheckerSlot::default());
    let resolver = Resolver::new((*checker).get_emit_resolver(), slot);
    let options = program.options();
    slot.lend(&mut checker, || {
        let context = printer::new_emit_context();
        let opts = TransformOptions { context, compiler_options: options, resolver: ReferenceResolverRef::Emit(resolver), emit_resolver: resolver };
        let mut result = source_file;
        for t in transforms {
            result = t(&opts).transform_source_file(result);
        }
        let mut p = printer::new_printer(printer::PrinterOptions { target: options.get_emit_script_target(), ..Default::default() }, printer::PrintHandlers::default(), Some(context));
        p.emit_source_file(result)
    })
}

#[test]
fn jsx_react_jsx() {
    let out = emit(
        &[
            ("/src/tsconfig.json", r#"{"compilerOptions":{"noLib":true,"allowJs":true,"jsx":"react-jsx","module":"esnext","target":"esnext"},"files":["a.jsx"]}"#),
            ("/src/a.jsx", "export const x = <div className=\"a\" key=\"k\">hi &amp; {1}<span/></div>;\nexport const y = <>a</>;\n"),
        ],
        "/src/a.jsx",
        &[jsxtransforms::new_jsx_transformer],
    );
    println!("{out}");
    assert!(out.contains("import { Fragment as _Fragment, jsx as _jsx, jsxs as _jsxs } from \"react/jsx-runtime\";"), "{out}");
    assert!(out.contains("_jsxs(\"div\", { className: \"a\", children: [\"hi & \", 1, _jsx(\"span\", {})] }, \"k\")"), "{out}");
}

#[test]
fn jsx_react_classic() {
    let out = emit(
        &[
            ("/src/tsconfig.json", r#"{"compilerOptions":{"noLib":true,"allowJs":true,"jsx":"react","module":"esnext","target":"es2015"},"files":["a.jsx"]}"#),
            ("/src/a.jsx", "var React;\nconst x = <div {...p} a=\"1\">t<b/></div>;\n"),
        ],
        "/src/a.jsx",
        &[jsxtransforms::new_jsx_transformer],
    );
    println!("{out}");
    assert!(out.contains("React.createElement(\"div\", Object.assign({}, p, { a: \"1\" }),"), "{out}");
}

#[test]
fn legacy_decorators() {
    let out = emit(
        &[
            ("/src/tsconfig.json", r#"{"compilerOptions":{"noLib":true,"allowJs":true,"experimentalDecorators":true,"target":"es2015"},"files":["a.js"]}"#),
            ("/src/a.js", "function dec() {}\n@dec\nclass C {\n    static x() { return C.y; }\n    @dec m() {}\n}\n"),
        ],
        "/src/a.js",
        &[tstransforms::new_legacy_decorators_transformer],
    );
    println!("{out}");
    assert!(out.contains("let C = C_1 = class C {"), "{out}");
    assert!(out.contains("C = C_1 = __decorate([\n    dec\n], C);"), "{out}");
}
