use tsrs_modulespecifiers::{self as modulespecifiers, ModuleSpecifierOptions, ResultKind};

use super::export::Export;
use super::view::View;

impl View {
    // specifiers.go:9
    pub fn get_module_specifier(&self, export: &Export, user_preferences: &modulespecifiers::UserPreferences) -> (String, ResultKind) {
        if !export.unresolved_module_specifier.is_empty() {
            let mut specifier = export.unresolved_module_specifier.clone();
            if tsrs_core::tspath::path_is_relative(&specifier) {
                specifier = tsrs_core::tspath::get_relative_path_from_directory(&tsrs_core::tspath::get_directory_path(self.importing_file.file_name()), &export.module_file_name, &tsrs_core::tspath::ComparePathsOptions {
                    use_case_sensitive_file_names: self.program.use_case_sensitive_file_names(), current_directory: String::new(),
                });
                if tsrs_core::tspath::path_is_absolute(&specifier) { return (String::new(), ResultKind::None); }
                specifier = tsrs_core::tspath::ensure_path_is_non_module_name(&specifier);
            }
            if modulespecifiers::is_excluded_by_regex(&specifier, &user_preferences.auto_import_specifier_exclude_regexes) { return (String::new(), ResultKind::None); }
            return (specifier, ResultKind::Relative);
        }
        if export.module_id.is_ambient() {
            let specifier = export.module_id.as_str();
            if modulespecifiers::is_excluded_by_regex(specifier, &user_preferences.auto_import_specifier_exclude_regexes) {
                return (String::new(), ResultKind::None);
            }
            return (export.module_id.as_str().to_string(), ResultKind::Ambient);
        }

        let registry = self.registry();
        if !export.package_name.is_empty() {
            if let Some(entrypoints) = registry.entrypoints.get(&export.path) {
                if let Some(result) = entrypoints.iter().find_map(|entrypoint| {
                    let include_is_subset = entrypoint.include_conditions.as_ref().is_none_or(|i| i.is_subset_of(&self.conditions));
                    let excluded = entrypoint.exclude_conditions.as_ref().is_some_and(|e| self.conditions.intersects(e));
                    if include_is_subset && !excluded {
                        let specifier = modulespecifiers::process_entrypoint_ending(
                            entrypoint,
                            user_preferences,
                            self.program,
                            &self.program.options(),
                            self.importing_file,
                            self.get_allowed_endings(),
                        );

                        if !modulespecifiers::is_excluded_by_regex(&specifier, &user_preferences.auto_import_specifier_exclude_regexes) {
                            return Some((specifier, ResultKind::NodeModules));
                        }
                    }

                    None
                }) {
                    return result;
                }
                return (String::new(), ResultKind::None);
            }
        }

        // Go indexes the map; a missing cache is a nil *SyncMap whose Load and Store dereference nil.
        let cache = registry.specifier_cache.get(&self.importing_file_path).expect("nil specifier cache");
        if export.package_name.is_empty() {
            if let Some(specifier) = cache.load(&export.path) {
                if specifier.is_empty() {
                    return (String::new(), ResultKind::None);
                }
                return (specifier, ResultKind::Relative);
            }
        }

        let (specifiers, kind) = modulespecifiers::get_module_specifiers_for_file_with_info(
            self.importing_file,
            &export.module_file_name,
            &self.program.options(),
            self.program,
            user_preferences,
            ModuleSpecifierOptions::default(),
            true,
        );
        // !!! unsure when this could return multiple specifiers combined with the
        //     new node_modules code. Possibly with local symlinks, which should be
        //     very rare.
        if let Some(result) = specifiers.into_iter().find_map(|specifier| {
            if specifier.contains("/node_modules/") {
                return None;
            }
            cache.store(export.path.clone(), specifier.clone());
            Some((specifier, kind))
        }) {
            return result;
        }
        cache.store(export.path.clone(), String::new());
        (String::new(), ResultKind::None)
    }
}
