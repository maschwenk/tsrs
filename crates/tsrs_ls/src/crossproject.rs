// crossproject.go: the interfaces (the multi-project orchestration functions serve references, rename,
// implementations and call hierarchy, which are phase 3).

use std::sync::Arc;

use tsrs_compiler::Program;
use tsrs_core::collections::Set;
use tsrs_core::context::Context;
use tsrs_core::tspath::Path;
use tsrs_lsproto as lsproto;

use crate::languageservice::LanguageService;

// crossproject.go:17
pub trait Project: Send + Sync {
    fn id(&self) -> String;
    fn get_program(&self) -> Option<&'static Program>;
    fn has_file(&self, file_name: &str) -> bool;
}

// crossproject.go:38
pub trait CrossProjectOrchestrator: Send + Sync {
    fn get_default_project(&self) -> Arc<dyn Project>;
    fn get_all_projects_for_initial_request(&self) -> Vec<Arc<dyn Project>>;
    fn get_language_service_for_project_with_file(&self, ctx: &Context, project: &Arc<dyn Project>, uri: &lsproto::DocumentUri) -> Option<Arc<LanguageService>>;
    fn get_projects_for_file(&self, ctx: &Context, uri: &lsproto::DocumentUri) -> Result<Vec<Arc<dyn Project>>, lsproto::Error>;
    fn get_projects_loading_project_tree(&self, ctx: &Context, requested_project_trees: &Set<Path>) -> Box<dyn Iterator<Item = Arc<dyn Project>> + '_>;
}
