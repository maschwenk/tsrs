use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use rustc_hash::FxHashSet;
use tsrs_core::Tristate;
use tsrs_core::tspath::{self, Path};
use tsrs_lsproto::PositionEncodingKind;
use tsrs_vfs::FS;

use crate::configfileregistry::ConfigFileRegistry;
use crate::configfileregistrybuilder::{
    configFileRegistryBuilder, new_config_file_registry_builder,
};
use crate::extendedconfigcache::new_extended_config_cache;
use crate::logging::LogTree;
use crate::overlayfs::{FsRef, ToPath, layer_overlay_file_system};
use crate::projectcollectionbuilder::projectLoadKind;
use crate::session::SessionOptions;
use crate::snapshotfs::new_snapshot_fs_builder_from_source;

pub struct TsConfigResolver {
    fs: Arc<dyn FS>,
    current_directory: String,
    builder: configFileRegistryBuilder,
    logger: LogTree,
}

impl TsConfigResolver {
    pub fn new(fs: Arc<dyn FS>, current_directory: &str) -> TsConfigResolver {
        let current_directory = tspath::normalize_path(current_directory);
        let case_sensitive = fs.use_case_sensitive_file_names();
        let cwd = current_directory.clone();
        let to_path: ToPath =
            Arc::new(move |file_name| tspath::to_path(file_name, &cwd, case_sensitive));
        let layered = layer_overlay_file_system(
            &FsRef::Host(Arc::clone(&fs)),
            Arc::default(),
            PositionEncodingKind::UTF16,
            Arc::clone(&to_path),
        );
        let snapshot = Arc::new(new_snapshot_fs_builder_from_source(
            layered,
            Arc::default(),
            Arc::default(),
            Arc::default(),
            to_path,
        ));
        let session_options = Arc::new(SessionOptions {
            current_directory: current_directory.clone(),
            ..Default::default()
        });
        let logger = LogTree::nil();
        let builder = new_config_file_registry_builder(
            false,
            snapshot,
            Box::new(|_| true),
            Arc::new(ConfigFileRegistry::default()),
            Arc::new(new_extended_config_cache()),
            0,
            session_options,
            "",
            &logger,
        );
        TsConfigResolver {
            fs,
            current_directory,
            builder,
            logger,
        }
    }

    fn to_path(&self, file_name: &str) -> Path {
        tspath::to_path(
            file_name,
            &self.current_directory,
            self.fs.use_case_sensitive_file_names(),
        )
    }

    fn search_config_graph(
        &self,
        file_name: &str,
        file_path: &Path,
        root_config: &str,
    ) -> Option<String> {
        let mut queue = VecDeque::from([root_config.to_string()]);
        let mut visited = FxHashSet::default();
        while let Some(config_name) = queue.pop_front() {
            let config_path = self.to_path(&config_name);
            if !visited.insert(config_path.clone()) {
                continue;
            }
            let Some(config) = self.builder.find_or_acquire_config_for_file(
                &config_name,
                &config_path,
                file_path,
                projectLoadKind::Create,
                &self.logger,
            ) else {
                continue;
            };
            if config.file_names().is_empty() {
                queue.extend(config.resolved_project_reference_paths().iter().cloned());
                continue;
            }
            if config.compiler_options().unwrap().composite == Tristate::True
                && !config.possibly_matches_file_name(file_name)
            {
                queue.extend(config.resolved_project_reference_paths().iter().cloned());
                continue;
            }
            if config.file_names_by_path().contains_key(file_path) {
                return Some(config_name);
            }
            queue.extend(config.resolved_project_reference_paths().iter().cloned());
        }
        None
    }

    pub fn find_tsconfig_for_file(&self, file_name: &str) -> Option<String> {
        let normalized = tspath::normalize_slashes(file_name);
        let path = self.to_path(&normalized);
        let mut config_name =
            self.builder
                .compute_config_file_name(&normalized, false, &self.logger);
        while !config_name.is_empty() {
            if let Some(result) = self.search_config_graph(&normalized, &path, &config_name) {
                return Some(result);
            }
            let config_path = self.to_path(&config_name);
            let disable_solution_searching = self
                .builder
                .find_or_acquire_config_for_file(
                    &config_name,
                    &config_path,
                    &path,
                    projectLoadKind::Create,
                    &self.logger,
                )
                .is_some_and(|c| {
                    c.compiler_options()
                        .unwrap()
                        .disable_solution_searching
                        .is_true()
                });
            if disable_solution_searching {
                break;
            }
            let ancestor = self.builder.get_ancestor_config_file_name(
                &normalized,
                &path,
                &config_name,
                &self.logger,
            );
            config_name = if ancestor.is_empty() {
                self.builder
                    .compute_config_file_name(&config_name, true, &self.logger)
            } else {
                ancestor
            };
        }
        None
    }

    pub fn find_tsconfigs(&self, file_names: &[String]) -> BTreeMap<String, Option<String>> {
        file_names
            .iter()
            .map(|file| (file.clone(), self.find_tsconfig_for_file(file)))
            .collect()
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use tsrs_vfs::{FS, vfstest};

    use super::TsConfigResolver;

    fn resolver(files: &[(&str, &str)]) -> TsConfigResolver {
        let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(files.iter().copied(), true));
        TsConfigResolver::new(fs, "/repo")
    }

    #[test]
    fn finds_nearest_config_and_inferred_misses() {
        let resolver = resolver(&[
            ("/repo/tsconfig.json", r#"{ "include": ["src/**/*.ts"] }"#),
            ("/repo/src/included.ts", "export {};"),
            ("/repo/outside.ts", "export {};"),
        ]);
        assert_eq!(
            resolver.find_tsconfig_for_file("/repo/src/included.ts"),
            Some("/repo/tsconfig.json".to_string())
        );
        assert_eq!(resolver.find_tsconfig_for_file("/repo/outside.ts"), None);
        assert_eq!(resolver.find_tsconfig_for_file("/repo/missing.ts"), None);
    }

    #[test]
    fn uses_ancestor_when_the_nearest_config_excludes_a_file() {
        let resolver = resolver(&[
            ("/repo/tsconfig.json", r#"{ "include": ["**/*.ts"] }"#),
            (
                "/repo/packages/cli/tsconfig.json",
                r#"{ "files": ["src/kept.ts"] }"#,
            ),
            ("/repo/packages/cli/src/kept.ts", "export {};"),
            ("/repo/packages/cli/src/excluded.ts", "export {};"),
        ]);
        assert_eq!(
            resolver.find_tsconfig_for_file("/repo/packages/cli/src/kept.ts"),
            Some("/repo/packages/cli/tsconfig.json".to_string())
        );
        assert_eq!(
            resolver.find_tsconfig_for_file("/repo/packages/cli/src/excluded.ts"),
            Some("/repo/tsconfig.json".to_string())
        );
    }

    #[test]
    fn follows_solution_project_references() {
        let resolver = resolver(&[
            (
                "/repo/tsconfig.json",
                r#"{ "files": ["root.ts"], "references": [{ "path": "./project" }] }"#,
            ),
            ("/repo/root.ts", "export {};"),
            (
                "/repo/project/tsconfig.json",
                r#"{ "files": ["../shared.ts"] }"#,
            ),
            ("/repo/shared.ts", "export {};"),
        ]);
        assert_eq!(
            resolver.find_tsconfig_for_file("/repo/shared.ts"),
            Some("/repo/project/tsconfig.json".to_string())
        );
    }
}
