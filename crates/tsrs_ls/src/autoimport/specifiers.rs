use tsrs_modulespecifiers::{self as modulespecifiers, ModuleSpecifierOptions, ResultKind};

use super::export::Export;
use super::view::View;

impl View {
    // specifiers.go:9
    pub fn get_module_specifier(&self, export: &Export, user_preferences: &modulespecifiers::UserPreferences) -> (String, ResultKind) {
        // Ambient module
        if modulespecifiers::path_is_bare_specifier(&export.module_id.0) {
            let specifier = &export.module_id.0;
            if modulespecifiers::is_excluded_by_regex(specifier, &user_preferences.auto_import_specifier_exclude_regexes) {
                return (String::new(), ResultKind::None);
            }
            return (export.module_id.0.clone(), ResultKind::Ambient);
        }

        let registry = self.registry();
        if !export.package_name.is_empty() {
            if let Some(entrypoints) = registry.entrypoints.get(&export.path) {
                for entrypoint in entrypoints {
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
                            return (specifier, ResultKind::NodeModules);
                        }
                    }
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
            user_preferences.clone(),
            ModuleSpecifierOptions::default(),
            true,
        );
        // !!! unsure when this could return multiple specifiers combined with the
        //     new node_modules code. Possibly with local symlinks, which should be
        //     very rare.
        for specifier in specifiers {
            if specifier.contains("/node_modules/") {
                continue;
            }
            cache.store(export.path.clone(), specifier.clone());
            return (specifier, kind);
        }
        cache.store(export.path.clone(), String::new());
        (String::new(), ResultKind::None)
    }
}
