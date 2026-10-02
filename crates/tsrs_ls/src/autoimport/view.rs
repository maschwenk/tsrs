// PLACEHOLDER (see mod.rs): view.go, reduced to what phase-1 ls calls.

use std::sync::Arc;

use tsrs_ast::SourceFile;
use tsrs_checker::Checker;
use tsrs_compiler::Program;
use tsrs_core::tspath::Path;
use tsrs_core::P;

use super::{ProjectID, Registry};

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
