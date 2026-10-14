use std::sync::Arc;

use tsrs_ast::{self as ast, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{self as checker, Checker};
use tsrs_core::tspath::Path;
use tsrs_core::P;

use super::extract::new_symbol_extractor;
use super::util::try_get_module_id_and_file_name_of_module_symbol;
use crate::lsutil::{ScriptElementKind, ScriptElementKindModifier};

// export.go:17
// ModuleID distinguishes file modules from ambient module specifiers.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum ModuleID {
    #[default]
    Invalid,
    File(String),
    Ambient(String),
}
impl ModuleID {
    pub fn as_str(&self) -> &str { match self { Self::Invalid => "", Self::File(s) | Self::Ambient(s) => s } }
    pub fn is_ambient(&self) -> bool { matches!(self, Self::Ambient(_)) }
}
impl std::fmt::Display for ModuleID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
}

// export.go:19
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ExportID {
    pub module_id: ModuleID,
    pub export_name: String,
}

// export.go:24
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum ExportSyntax {
    #[default]
    None,
    // export const x = {}
    Modifier,
    // export { x }
    Named,
    // export default function f() {}
    DefaultModifier,
    // export default f
    DefaultDeclaration,
    // export = x
    Equals,
    // export as namespace x
    #[expect(clippy::upper_case_acronyms, reason = "The name matches the upstream Go or protocol vocabulary.")]
    UMD,
    // export * from "module"
    Star,
    // module.exports = {}
    CommonJSModuleExports,
    // exports.x = {}
    CommonJSExportsProperty,
}

impl ExportSyntax {
    // export_stringer_generated.go
    pub fn string(self) -> String {
        format!("ExportSyntax{self:?}")
    }
}

// export.go:48
#[derive(Clone, Default, Debug)]
pub struct Export {
    pub export_id: ExportID,
    pub module_file_name: String,
    pub unresolved_module_specifier: String,
    pub syntax: ExportSyntax,
    pub flags: SymbolFlags,
    pub(crate) local_name: String,
    // through is the name of the module symbol's export that this export was found on,
    // either 'export=', InternalSymbolNameExportStar, or empty string.
    pub(crate) through: String,

    // Checker-set fields
    pub target: ExportID,
    pub is_type_only: bool,
    pub script_element_kind: ScriptElementKind,
    pub script_element_kind_modifiers: ScriptElementKindModifier,

    // The file where the export was found.
    pub path: Path,

    pub package_name: String,
}

impl std::ops::Deref for Export {
    type Target = ExportID;
    fn deref(&self) -> &ExportID {
        &self.export_id
    }
}

impl Export {
    // export.go:71
    pub fn name(&self) -> &str {
        if !self.local_name.is_empty() {
            return &self.local_name;
        }
        if self.export_name == ast::InternalSymbolNameExportEquals {
            return &self.target.export_name;
        }
        &self.export_name
    }

    // export.go:81
    pub fn is_renameable(&self) -> bool {
        self.export_name == ast::InternalSymbolNameExportEquals || self.export_name == ast::InternalSymbolNameDefault
    }

    // export.go:85
    pub fn ambient_module_name(&self) -> &str {
        if self.module_id.is_ambient() {
            return self.module_id.as_str();
        }
        ""
    }

    // export.go:92
    pub fn is_unresolved_alias(&self) -> bool {
        self.flags == SymbolFlags::Alias
    }
}

impl super::index::Named for Arc<Export> {
    fn name(&self) -> &str {
        Export::name(self)
    }
}

// export.go:96
pub fn symbol_to_export(symbol: P<Symbol>, ch: &mut Checker) -> Option<Arc<Export>> {
    if let Some(parent) = symbol.parent() {
        if checker::is_external_module_symbol(parent) {
            if let Some((module_id, module_file_name)) = try_get_module_id_and_file_name_of_module_symbol(parent) {
                return extract_first_export(symbol, ch, &module_id, &module_file_name, ast::get_source_file_of_module(parent).unwrap());
            }
            return None;
        }
    }

    let declaration = symbol.declarations().first().copied()?;

    let file = ast::get_source_file_of_node(declaration).unwrap();
    let file_symbol = file.symbol()?;

    let module_symbol = ch.get_merged_symbol(file_symbol);
    let module_id = ModuleID::File(file.path().to_string());
    let module_file_name = file.file_name().to_string();
    let skipped = ch.skip_alias(symbol);
    let target = ch.get_merged_symbol(skipped);

    if let Some(export) = try_get_module_export(ast::InternalSymbolNameDefault, target, module_symbol, ch, &module_id, &module_file_name, file) {
        return Some(export);
    }
    if let Some(export) = try_get_module_export(ast::InternalSymbolNameExportEquals, target, module_symbol, ch, &module_id, &module_file_name, file) {
        return Some(export);
    }
    try_get_module_export(symbol.name(), target, module_symbol, ch, &module_id, &module_file_name, file)
}

// export.go:125
fn try_get_module_export(
    export_name: &str,
    target: P<Symbol>,
    module_symbol: P<Symbol>,
    ch: &mut Checker,
    module_id: &ModuleID,
    module_file_name: &str,
    file: P<SourceFile>,
) -> Option<Arc<Export>> {
    let exported = ch.try_get_member_in_module_exports_and_properties(export_name, module_symbol);
    if let Some(exported) = exported {
        let skipped = ch.skip_alias(exported);
        if ch.get_merged_symbol(skipped) == target {
            return extract_first_export(exported, ch, module_id, module_file_name, file);
        }
    }
    None
}

// export.go:133
fn extract_first_export(symbol: P<Symbol>, ch: &mut Checker, module_id: &ModuleID, module_file_name: &str, file: P<SourceFile>) -> Option<Arc<Export>> {
    let mut exports: Vec<Arc<Export>> = Vec::new();
    let mut extractor = new_symbol_extractor("", ch, None, None);
    extractor.extract_from_symbol(symbol.name(), symbol, module_id, module_file_name, file, &mut exports);
    exports.into_iter().next()
}
