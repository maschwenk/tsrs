// PLACEHOLDER (see mod.rs): export.go, reduced to the fields and methods completions read.

use tsrs_ast::SymbolFlags;
use tsrs_core::tspath::Path;

use crate::lsutil::{ScriptElementKind, ScriptElementKindModifier};

// export.go:19 (placeholder: the module id is the string Go stores)
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ExportID {
    pub module_id: String,
    pub export_name: String,
}

// export.go:24
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
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
    pub fn string(&self) -> String {
        format!("ExportSyntax{:?}", self)
    }
}

// export.go:48
#[derive(Clone, Default, Debug)]
pub struct Export {
    pub export_id: ExportID,
    pub module_file_name: String,
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
    // export.go:92
    pub fn is_unresolved_alias(&self) -> bool {
        self.flags == SymbolFlags::Alias
    }
}
