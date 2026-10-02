// PLACEHOLDER (see mod.rs): registry.go, reduced to what project and phase-1 ls call.

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::SourceFile;
use tsrs_compiler::Program;
use tsrs_core::collections::Set;
use tsrs_core::context::Context;
use tsrs_core::tspath::Path;
use tsrs_core::P;
use tsrs_lsproto as lsproto;
use tsrs_module::packagejson::InfoCacheEntry;
use tsrs_module::ResolutionHost;

use crate::lsutil::UserPreferences;

// registry.go:32
// Go `interface { fmt.Stringer }` used as a map key. Its only implementation is `project.ID` (a string), so the
// interface value is that string; project converts with `ProjectID(id.0)` / `ID(project_id.0)`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct ProjectID(pub String);

impl ProjectID {
    pub fn string(&self) -> String {
        self.0.clone()
    }
}

impl std::fmt::Display for ProjectID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type ToPath = Arc<dyn Fn(&str) -> Path + Send + Sync>;

// registry.go:328 (placeholder: no directories or caches; a project bucket is the set of projects whose (empty)
// bucket has been built)
pub struct Registry {
    to_path: ToPath,
    user_preferences: UserPreferences,
    projects: FxHashSet<ProjectID>,
}

// registry.go:346
pub fn new_registry(to_path: ToPath, preferences: UserPreferences) -> Arc<Registry> {
    Arc::new(Registry { to_path, user_preferences: preferences, projects: FxHashSet::default() })
}

// Go's nil-receiver `(*Registry).IsPreparedForImportingFile` (registry.go:354): a nil registry is never prepared.
pub trait RegistryExt {
    fn is_prepared_for_importing_file(&self, file_name: &str, project_id: &ProjectID, preferences: &UserPreferences) -> bool;
}

impl RegistryExt for Option<Arc<Registry>> {
    fn is_prepared_for_importing_file(&self, file_name: &str, project_id: &ProjectID, preferences: &UserPreferences) -> bool {
        match self {
            None => false,
            Some(r) => r.is_prepared_for_importing_file(file_name, project_id, preferences),
        }
    }
}

impl Registry {
    // registry.go:354 (placeholder: an empty bucket never needs a rebuild and there are no node_modules buckets, so
    // the file is prepared once its project's bucket exists)
    pub fn is_prepared_for_importing_file(&self, _file_name: &str, project_id: &ProjectID, _preferences: &UserPreferences) -> bool {
        self.projects.contains(project_id)
    }

    // registry.go:383 (placeholder: no directories)
    pub fn node_modules_directories(&self) -> FxHashMap<Path, String> {
        FxHashMap::default()
    }

    // registry.go:393 (placeholder: keeps Go's user-preference update; drops the buckets of projects whose program
    // structure changed and builds an empty bucket for the requested file's default project, as Go's builder does
    // for a requested file, without extracting any exports)
    // Go's logger is `*project/logging.LogTree`; that package lives below `project` in Go but the Rust port of it is
    // in `tsrs_project`, which depends on this crate, so the placeholder takes any logger value.
    pub fn clone_registry<L>(&self, _ctx: &Context, change: RegistryChange, host: &dyn RegistryCloneHost, _logger: L) -> Result<Arc<Registry>, String> {
        let mut user_preferences = self.user_preferences.clone();
        if let Some(prefs) = change.user_preferences {
            user_preferences = prefs;
        }
        let mut projects = self.projects.clone();
        for project_id in change.rebuilt_programs.keys() {
            projects.remove(project_id);
        }
        if !change.requested_file.is_empty() {
            if let (Some(project_id), _) = host.get_default_project(&change.requested_file) {
                projects.insert(project_id);
            }
        }
        Ok(Arc::new(Registry { to_path: self.to_path.clone(), user_preferences, projects }))
    }

    pub fn to_path(&self, file_name: &str) -> Path {
        (self.to_path)(file_name)
    }
}

// registry.go:487
#[derive(Default)]
pub struct RegistryChange {
    pub requested_file: Path,
    pub open_files: FxHashMap<Path, String>,
    pub changed: Set<lsproto::DocumentUri>,
    pub created: Set<lsproto::DocumentUri>,
    pub deleted: Set<lsproto::DocumentUri>,
    // RebuiltPrograms maps from project ID to:
    //   - true: the program was rebuilt with a different set of file names
    //   - false: the program was rebuilt but the set of file names is unchanged
    pub rebuilt_programs: FxHashMap<ProjectID, bool>,
    pub user_preferences: Option<UserPreferences>,
}

// registry.go:500 (Go embeds module.ResolutionHost and repeats its FS(); `ResolutionHost::fs` covers both)
pub trait RegistryCloneHost: ResolutionHost {
    fn get_default_project(&self, path: &Path) -> (Option<ProjectID>, Option<&'static Program>);
    fn get_program_for_project(&self, project_id: &ProjectID) -> Option<&'static Program>;
    fn get_package_json(&self, file_name: &str) -> P<InfoCacheEntry>;
    fn get_source_file(&self, file_name: &str, path: &Path) -> Option<P<SourceFile>>;
    fn dispose(&self);
}
