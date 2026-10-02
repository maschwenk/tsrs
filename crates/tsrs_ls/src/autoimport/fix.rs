// PLACEHOLDER (see mod.rs): fix.go, reduced to what completions call.

use tsrs_ast::{self as ast, SourceFile};
use tsrs_compiler::Program;
use tsrs_core::context::Context;
use tsrs_core::{CompilerOptions, ModuleKind, P};
use tsrs_lsproto as lsproto;

use super::{Export, ExportSyntax};
use crate::lsconv::Converters;
use crate::lsutil::{FormatCodeSettings, UserPreferences};

// fix.go:37 (placeholder: only the embedded lsproto fix; nothing builds a Fix until the registry is ported)
#[derive(Clone, Default, Debug)]
pub struct Fix {
    pub auto_import_fix: lsproto::AutoImportFix,
}

impl std::ops::Deref for Fix {
    type Target = lsproto::AutoImportFix;
    fn deref(&self) -> &lsproto::AutoImportFix {
        &self.auto_import_fix
    }
}

impl Fix {
    // fix.go:56 (placeholder: the edit computation is not ported; returns no edits and ok = false. The language
    // service only calls it for completion items carrying auto-import data, which it never produces until the
    // registry is ported.)
    pub fn edits(
        &self,
        _ctx: &Context,
        _file: P<SourceFile>,
        _compiler_options: P<CompilerOptions>,
        _format_options: FormatCodeSettings,
        _converters: &Converters,
        _preferences: &UserPreferences,
    ) -> (Vec<lsproto::TextEdit>, String, bool) {
        (Vec::new(), String::new(), false)
    }
}

// fix.go:796
pub fn get_import_kind_for_import_statement(importing_file: P<SourceFile>, export: &Export, program: &Program) -> lsproto::ImportKind {
    get_import_kind(importing_file, export, program, true /*forceImportKeyword*/)
}

// fix.go:800
fn get_import_kind(importing_file: P<SourceFile>, export: &Export, program: &Program, force_import_keyword: bool) -> lsproto::ImportKind {
    if program.options().verbatim_module_syntax.is_true() && program.get_emit_module_format_of_file(importing_file) == ModuleKind::CommonJS {
        return lsproto::ImportKind::CommonJS;
    }
    match export.syntax {
        ExportSyntax::DefaultModifier | ExportSyntax::DefaultDeclaration => lsproto::ImportKind::Default,
        ExportSyntax::Named | ExportSyntax::Modifier | ExportSyntax::Star | ExportSyntax::CommonJSExportsProperty => {
            if export.syntax == ExportSyntax::Named && export.export_name == ast::InternalSymbolNameDefault {
                return lsproto::ImportKind::Default;
            }
            lsproto::ImportKind::Named
        }
        ExportSyntax::Equals | ExportSyntax::CommonJSModuleExports | ExportSyntax::UMD => {
            // export.Syntax will be ExportSyntaxEquals for named exports/properties of an export='s target.
            if export.export_name != ast::InternalSymbolNameExportEquals {
                return lsproto::ImportKind::Named;
            }
            // !!! cache this?
            for &statement in importing_file.statements.nodes() {
                // `import foo` parses as an ImportEqualsDeclaration even though it could be an ImportDeclaration
                if ast::is_import_equals_declaration(statement) && !ast::node_is_missing(Some(statement.as_import_equals_declaration().module_reference)) {
                    return lsproto::ImportKind::CommonJS;
                }
            }
            // !!! this logic feels weird; we're basically trying to predict if shouldUseRequire is going to
            //     be true. The meaning of "default import" is different depending on whether we write it as
            //     a require or an es6 import. The latter, compiled to CJS, has interop built in that will
            //     avoid accessing .default, but if we write a require directly and call it a default import,
            //     we emit an unconditional .default access.
            if importing_file.external_module_indicator().is_some() || force_import_keyword || !ast::is_source_file_js(importing_file) {
                return lsproto::ImportKind::Default;
            }
            lsproto::ImportKind::CommonJS
        }
        _ => panic!("unhandled export syntax kind: {}", export.syntax.string()),
    }
}
