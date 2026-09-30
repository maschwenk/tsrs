use std::sync::OnceLock;

use rustc_hash::FxHashMap;
use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_core::collections::OrderedMap;
use tsrs_core::glob::{self, Glob};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{resolve_project_reference_path, CompilerOptions, ProjectReference, TypeAcquisition, P};
use tsrs_vfs::vfsmatch;
use tsrs_vfs::FS;

use crate::commandlineoption::CompilerOptionsValue;
use crate::contentmappers::Mapper;
use crate::parsedoptions::ParsedOptions;
use crate::tsconfigparsing::{get_file_names_from_config_specs, TsConfigSourceFile};
use crate::wildcarddirectories::get_wildcard_directories;

const FILE_GLOB_PATTERN: &str = "*.{js,jsx,mjs,cjs,ts,tsx,mts,cts,json}";
const RECURSIVE_FILE_GLOB_PATTERN: &str = "**/*.{js,jsx,mjs,cjs,ts,tsx,mts,cts,json}";

impl ParsedCommandLine {
    // fileGlobPatterns returns the include file glob patterns for this command line, augmenting the
    // built-in patterns with the extensions registered by its content mappers so that created
    // content-mapped files are recognized as possible root files.
    pub(crate) fn file_glob_patterns(&self) -> (String, String) {
        let mapper_extensions = self.content_mapper_extensions();
        if mapper_extensions.is_empty() {
            return (FILE_GLOB_PATTERN.to_string(), RECURSIVE_FILE_GLOB_PATTERN.to_string());
        }
        let mut extensions: Vec<&str> = Vec::with_capacity(9 + mapper_extensions.len());
        extensions.extend(["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "json"]);
        for extension in &mapper_extensions {
            extensions.push(extension.strip_prefix('.').unwrap_or(extension));
        }
        let file_glob = format!("*.{{{}}}", extensions.join(","));
        let recursive = format!("**/{file_glob}");
        (file_glob, recursive)
    }
}

pub struct ParsedCommandLine {
    pub parsed_config: ParsedOptions,

    pub config_file: Option<P<TsConfigSourceFile>>, // TsConfigSourceFile, used in Program and ExecuteCommandLine
    pub errors: Vec<P<Diagnostic>>,
    pub raw: CompilerOptionsValue,
    pub compile_on_save: Option<bool>,

    pub(crate) compare_paths_options: ComparePathsOptions,
    pub(crate) wildcard_directories: OnceLock<Option<OrderedMap<String, bool>>>,
    pub(crate) include_globs: OnceLock<Vec<Glob>>,

    pub(crate) resolved_project_reference_paths: OnceLock<Vec<String>>,

    pub(crate) literal_file_names_len: usize,
    pub(crate) file_names_by_path: OnceLock<FxHashMap<Path, String>>, // maps file names to their paths, used for quick lookups
}

impl ParsedCommandLine {
    pub(crate) fn empty() -> ParsedCommandLine {
        ParsedCommandLine {
            parsed_config: ParsedOptions::default(),
            config_file: None,
            errors: Vec::new(),
            raw: CompilerOptionsValue::Null,
            compile_on_save: None,
            compare_paths_options: ComparePathsOptions::default(),
            wildcard_directories: OnceLock::new(),
            include_globs: OnceLock::new(),
            resolved_project_reference_paths: OnceLock::new(),
            literal_file_names_len: 0,
            file_names_by_path: OnceLock::new(),
        }
    }
}

pub fn new_parsed_command_line(
    compiler_options: P<CompilerOptions>,
    root_file_names: Vec<String>,
    project_references: Vec<ProjectReference>,
    compare_paths_options: ComparePathsOptions,
) -> ParsedCommandLine {
    ParsedCommandLine {
        parsed_config: ParsedOptions {
            compiler_options: Some(compiler_options),
            file_names: root_file_names,
            project_references,
            ..Default::default()
        },
        compare_paths_options,
        ..ParsedCommandLine::empty()
    }
}

fn clone_once_lock<T: Clone>(lock: &OnceLock<T>) -> OnceLock<T> {
    let result = OnceLock::new();
    if let Some(value) = lock.get() {
        let _ = result.set(value.clone());
    }
    result
}

impl ParsedCommandLine {
    pub fn with_file_names(&self, file_names: Vec<String>) -> ParsedCommandLine {
        let mut parsed_config = self.parsed_config.clone();
        parsed_config.file_names = file_names;
        ParsedCommandLine {
            parsed_config,
            config_file: self.config_file,
            errors: self.errors.clone(),
            raw: self.raw.clone(),
            compile_on_save: self.compile_on_save,
            compare_paths_options: self.compare_paths_options.clone(),
            wildcard_directories: clone_once_lock(&self.wildcard_directories),
            include_globs: clone_once_lock(&self.include_globs),
            literal_file_names_len: self.literal_file_names_len,
            ..ParsedCommandLine::empty()
        }
    }

    pub fn config_name(&self) -> &str {
        match self.config_file {
            None => "",
            Some(config_file) => config_file.get().source_file.get().file_name(),
        }
    }

    pub fn get_current_directory(&self) -> &str {
        &self.compare_paths_options.current_directory
    }

    pub fn use_case_sensitive_file_names(&self) -> bool {
        self.compare_paths_options.use_case_sensitive_file_names
    }

    // WildcardDirectories returns the cached wildcard directories, initializing them if needed
    pub fn wildcard_directories(&self) -> Option<&OrderedMap<String, bool>> {
        self.wildcard_directories
            .get_or_init(|| {
                let config_file = self.config_file.unwrap();
                let specs = config_file.config_file_specs.borrow();
                let specs = specs.as_ref().unwrap();
                get_wildcard_directories(&specs.validated_include_specs, &specs.validated_exclude_specs, &self.compare_paths_options)
            })
            .as_ref()
    }

    pub fn wildcard_directory_globs(&self) -> &[Glob] {
        let Some(wildcard_directories) = self.wildcard_directories() else {
            return &[];
        };

        self.include_globs.get_or_init(|| {
            let (file_glob, recursive_file_glob) = self.file_glob_patterns();
            let mut globs: Vec<Glob> = Vec::with_capacity(wildcard_directories.len());
            for (dir, recursive) in wildcard_directories.iter() {
                let pattern = format!(
                    "{}/{}",
                    tspath::normalize_path(dir),
                    if *recursive { &recursive_file_glob } else { &file_glob }
                );
                if let Ok(parsed) = glob::parse(&pattern) {
                    globs.push(parsed);
                }
            }
            globs
        })
    }

    // Normalized file names explicitly specified in `files`
    pub fn literal_file_names(&self) -> &[String] {
        if self.config_file.is_some() {
            return &self.file_names()[0..self.literal_file_names_len];
        }
        &[]
    }

    pub fn set_parsed_options(&mut self, o: ParsedOptions) {
        self.parsed_config = o;
    }

    pub fn set_compiler_options(&mut self, o: P<CompilerOptions>) {
        self.parsed_config.compiler_options = Some(o);
    }

    pub fn compiler_options(&self) -> Option<P<CompilerOptions>> {
        self.parsed_config.compiler_options
    }

    pub fn set_type_acquisition(&mut self, o: Option<TypeAcquisition>) {
        self.parsed_config.type_acquisition = o;
    }

    pub fn type_acquisition(&self) -> Option<&TypeAcquisition> {
        self.parsed_config.type_acquisition.as_ref()
    }

    // All file names matched by files, include, and exclude patterns
    pub fn file_names(&self) -> &[String] {
        &self.parsed_config.file_names
    }

    pub fn file_names_by_path(&self) -> &FxHashMap<Path, String> {
        self.file_names_by_path.get_or_init(|| {
            let mut file_names_by_path = FxHashMap::with_capacity_and_hasher(self.parsed_config.file_names.len(), Default::default());
            for file_name in &self.parsed_config.file_names {
                let path = tspath::to_path(file_name, self.get_current_directory(), self.use_case_sensitive_file_names());
                file_names_by_path.insert(path, file_name.clone());
            }
            file_names_by_path
        })
    }

    pub fn project_references(&self) -> &[ProjectReference] {
        &self.parsed_config.project_references
    }

    pub fn content_mappers(&self) -> &[Mapper] {
        &self.parsed_config.content_mappers
    }

    // ContentMapperExtensions returns the flattened list of file extensions registered by the
    // config's content mappers.
    pub fn content_mapper_extensions(&self) -> Vec<String> {
        self.content_mappers().iter().flat_map(|m| m.definition.extensions.iter().cloned()).collect()
    }

    // GetContentMapperForFileName returns the configured content mapper whose extensions include fileName,
    // or nil if no content mapper is registered for the file's extension.
    pub fn get_content_mapper_for_file_name(&self, file_name: &str) -> Option<&Mapper> {
        let ignore_case = !self.use_case_sensitive_file_names();
        let extensions = self.content_mapper_extensions();
        let extension_refs: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
        let extension = tspath::get_longest_extension_from_path(file_name, &extension_refs, ignore_case);
        self.content_mappers().iter().find(|mapper| {
            mapper.definition.extensions.iter().any(|mapper_extension| {
                extension == *mapper_extension || ignore_case && tsrs_core::stringutil::equal_fold(&extension, mapper_extension)
            })
        })
    }

    pub fn resolved_project_reference_paths(&self) -> &[String] {
        self.resolved_project_reference_paths
            .get_or_init(|| self.parsed_config.project_references.iter().map(resolve_project_reference_path).collect())
    }

    pub fn extended_source_files(&self) -> Vec<String> {
        match self.config_file {
            None => Vec::new(),
            Some(config_file) => config_file.extended_source_files.borrow().clone(),
        }
    }

    pub fn get_config_file_parsing_diagnostics(&self) -> Vec<P<Diagnostic>> {
        if let Some(config_file) = self.config_file {
            // todo: !!! should be ConfigFile.ParseDiagnostics, check if they are the same
            let mut result: Vec<P<Diagnostic>> = config_file.source_file.diagnostics().to_vec();
            result.extend(self.errors.iter().copied());
            return result;
        }
        self.errors.clone()
    }

    // PossiblyMatchesFileName is a fast check to see if a file is currently included by a config
    // or would be included if the file were to be created. It may return false positives.
    pub fn possibly_matches_file_name(&self, file_name: &str) -> bool {
        let path = tspath::to_path(file_name, self.get_current_directory(), self.use_case_sensitive_file_names());
        if self.file_names_by_path().contains_key(&path) {
            return true;
        }

        {
            let config_file = self.config_file.unwrap();
            let specs = config_file.config_file_specs.borrow();
            for include in &specs.as_ref().unwrap().validated_include_specs {
                if !include.contains(['*', '?']) && !vfsmatch::is_implicit_glob(include) {
                    let include_path = tspath::to_path(include, self.get_current_directory(), self.use_case_sensitive_file_names());
                    if include_path == path {
                        return true;
                    }
                }
            }
        }
        if self.get_content_mapper_for_file_name(file_name).is_some() {
            let directory_path = path.get_directory_path();
            if self.possibly_matches_directory_name(&directory_path) {
                return true;
            }
        }
        let wildcard_directory_globs = self.wildcard_directory_globs();
        if !wildcard_directory_globs.is_empty() {
            for glob in wildcard_directory_globs {
                if glob.match_(file_name) {
                    return true;
                }
            }
        }
        false
    }

    pub fn possibly_matches_directory_name(&self, directory_path: &Path) -> bool {
        let Some(wildcard_directories) = self.wildcard_directories() else {
            return false;
        };
        for (wildcard_dir, recursive) in wildcard_directories.iter() {
            let wildcard_dir_path = tspath::to_path(wildcard_dir, self.get_current_directory(), self.use_case_sensitive_file_names());
            if *recursive {
                if wildcard_dir_path.contains_path(directory_path) {
                    return true;
                }
            } else if wildcard_dir_path == *directory_path {
                return true;
            }
        }
        false
    }

    pub fn get_matched_file_spec(&self, file_name: &str) -> String {
        let config_file = self.config_file.unwrap();
        let specs = config_file.config_file_specs.borrow();
        specs.as_ref().unwrap().get_matched_file_spec(file_name, &self.compare_paths_options)
    }

    pub fn get_matched_include_spec(&self, file_name: &str) -> (String, bool) {
        let config_file = self.config_file.unwrap();
        let specs = config_file.config_file_specs.borrow();
        let specs = specs.as_ref().unwrap();
        if specs.validated_include_specs.is_empty() {
            return (String::new(), false);
        }

        if specs.is_default_include_spec {
            return (specs.validated_include_specs[0].clone(), true);
        }

        (specs.get_matched_include_spec(file_name, &self.compare_paths_options), false)
    }

    pub fn reload_file_names_of_parsed_command_line(&self, fs: &dyn FS) -> ParsedCommandLine {
        let mut parsed_config = self.parsed_config.clone();
        let config_file = self.config_file.unwrap();
        let (file_names, literal_file_names_len) = {
            let specs = config_file.config_file_specs.borrow();
            get_file_names_from_config_specs(
                specs.as_ref().unwrap(),
                self.get_current_directory(),
                self.compiler_options().as_deref(),
                fs,
                &self.content_mapper_extensions(),
            )
        };
        parsed_config.file_names = file_names;
        ParsedCommandLine {
            parsed_config,
            config_file: self.config_file,
            errors: self.errors.clone(),
            raw: self.raw.clone(),
            compile_on_save: self.compile_on_save,
            compare_paths_options: self.compare_paths_options.clone(),
            wildcard_directories: clone_once_lock(&self.wildcard_directories),
            include_globs: clone_once_lock(&self.include_globs),
            literal_file_names_len,
            ..ParsedCommandLine::empty()
        }
    }
}
