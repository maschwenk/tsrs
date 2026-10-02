// PLACEHOLDER (see mod.rs): view.go, reduced to what phase-1 ls calls.

use std::sync::Arc;

use tsrs_ast::SourceFile;
use tsrs_checker::Checker;
use tsrs_compiler::Program;
use tsrs_core::tspath::Path;
use tsrs_core::P;

use tsrs_lsproto as lsproto;

use super::{Export, Fix, ProjectID, Registry};

// view.go:21 (placeholder: the checker and the import caches are not kept; nothing reads them yet)
pub struct View {
    pub(crate) registry: Option<Arc<Registry>>,
    pub(crate) importing_file: P<SourceFile>,
    pub(crate) importing_file_path: Path,
    pub(crate) program: &'static Program,
    pub(crate) preferences: tsrs_modulespecifiers::UserPreferences,
    pub(crate) project_id: ProjectID,
}

// view.go:37
pub fn new_view(
    registry: Option<Arc<Registry>>,
    importing_file: P<SourceFile>,
    project_id: ProjectID,
    program: &'static Program,
    _type_checker: &mut Checker,
    preferences: tsrs_modulespecifiers::UserPreferences,
) -> View {
    let mut importing_file_path = importing_file.path().clone();
    if let Some(canonical) = importing_file.canonical_source_file() {
        importing_file_path = canonical.path().clone();
    }
    View { registry, importing_file, importing_file_path, program, preferences, project_id }
}

// view.go:175
pub struct FixAndExport {
    pub fix: Fix,
    pub export: Export,
}

impl View {
    // view.go:180 (placeholder: the registry holds no exports, so there is nothing to search)
    pub fn get_completions(&self, _prefix: &str, _position: lsproto::Position, _for_jsx: bool, _is_type_only_location: bool) -> Vec<FixAndExport> {
        Vec::new()
    }
}
