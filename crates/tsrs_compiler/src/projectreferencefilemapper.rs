#![forbid(unsafe_code)]

use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_core::collections::SyncMap;
use tsrs_core::tspath::{self, Path};
use tsrs_core::{CompilerOptions, P};
use tsrs_module::{self as module, ResolutionHost, ResolvedProjectReference};
use tsrs_tsoptions::{ParsedCommandLine, SourceOutputAndProjectReference};
use tsrs_vfs::FS;

use crate::host::CompilerHost;
use crate::projectreferencedtsfakinghost::new_project_reference_dts_faking_host;

pub(crate) struct projectReferenceFileMapper {
    data: Arc<projectReferenceRedirects>,
    // The host owns only redirect data, so caching it does not form an Arc cycle.
    dts_faking_host: OnceLock<Arc<dyn ResolutionHost>>,
}

pub(crate) struct projectReferenceRedirects {
    pub(crate) config: P<ParsedCommandLine>,
    pub(crate) use_source_of_project_reference: bool,
    pub(crate) dts_directories: FxHashSet<Path>,

    pub(crate) config_to_project_reference: FxHashMap<Path, Option<P<ParsedCommandLine>>>, // All the resolved references needed
    pub(crate) references_in_config_file: FxHashMap<Path, Vec<Path>>,                       // Map of config file to its references
    pub(crate) source_to_project_reference: FxHashMap<Path, P<SourceOutputAndProjectReference>>,
    pub(crate) output_dts_to_project_reference: FxHashMap<Path, P<SourceOutputAndProjectReference>>,

    // Store all the realpath from dts in node_modules to source file from project reference needed during parsing so it can be used later
    pub(crate) realpath_dts_to_source: SyncMap<Path, Option<P<SourceOutputAndProjectReference>>>,
}


pub(crate) struct projectReferenceFileMapperBuilder {
    pub(crate) mapper: Arc<projectReferenceFileMapper>,
    pub(crate) host: Arc<dyn ResolutionHost>,
}

struct compilerResolutionHost(Arc<dyn CompilerHost>);

impl ResolutionHost for compilerResolutionHost {
    fn fs(&self) -> &dyn FS {
        self.0.fs()
    }

    fn get_current_directory(&self) -> &str {
        self.0.get_current_directory()
    }
}

pub(crate) fn resolution_host_for(host: Arc<dyn CompilerHost>) -> Arc<dyn ResolutionHost> {
    Arc::new(compilerResolutionHost(host))
}

pub(crate) fn as_resolved_project_reference(config: P<ParsedCommandLine>) -> &'static dyn ResolvedProjectReference {
    config.get()
}

impl projectReferenceFileMapper {
    pub(crate) fn new(data: projectReferenceRedirects) -> Self {
        Self { data: Arc::new(data), dts_faking_host: OnceLock::new() }
    }

    // projectreferencefilemapper.go:30
    pub(crate) fn resolution_host(&self, host: Arc<dyn ResolutionHost>) -> Arc<dyn ResolutionHost> {
        if self.use_source_of_project_reference && !self.output_dts_to_project_reference.is_empty() {
            return Arc::clone(self.dts_faking_host.get_or_init(|| new_project_reference_dts_faking_host(host, Arc::clone(&self.data))));
        }
        host
    }
}

impl Deref for projectReferenceFileMapper {
    type Target = projectReferenceRedirects;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl projectReferenceRedirects {
    pub(crate) fn new(config: P<ParsedCommandLine>, use_source_of_project_reference: bool) -> Self {
        Self {
            config,
            use_source_of_project_reference,
            dts_directories: FxHashSet::default(),
            config_to_project_reference: FxHashMap::default(),
            references_in_config_file: FxHashMap::default(),
            source_to_project_reference: FxHashMap::default(),
            output_dts_to_project_reference: FxHashMap::default(),
            realpath_dts_to_source: SyncMap::default(),
        }
    }

    // projectreferencefilemapper.go:37
    pub(crate) fn root_config_path(&self) -> Path {
        match self.config.config_file {
            None => Path::new(""),
            Some(config_file) => config_file.source_file.path().clone(),
        }
    }

    // projectreferencefilemapper.go:44
    pub(crate) fn get_parse_file_redirect(&self, _file_name: &str, path: &Path) -> String {
        if self.use_source_of_project_reference {
            // Map to source file from project reference
            let mut source = self.get_project_reference_from_output_dts(path);
            if source.is_none() {
                source = self.realpath_dts_to_source.load(path).flatten();
            }
            if let Some(source) = source {
                return source.source.clone();
            }
        } else {
            // Map to dts file from project reference
            if let Some(output) = self.get_project_reference_from_source(path) {
                if !output.output_dts.is_empty() {
                    return output.output_dts.clone();
                }
            }
        }
        String::new()
    }

    // projectreferencefilemapper.go:64
    pub(crate) fn get_resolved_project_references(&self) -> Vec<Option<P<ParsedCommandLine>>> {
        let mut result = Vec::new();
        if let Some(refs) = self.references_in_config_file.get(&self.root_config_path()) {
            result.reserve(refs.len());
            for ref_path in refs {
                result.push(self.config_to_project_reference.get(ref_path).copied().flatten());
            }
        }
        result
    }

    // projectreferencefilemapper.go:77
    pub(crate) fn get_project_reference_from_source(&self, path: &Path) -> Option<P<SourceOutputAndProjectReference>> {
        self.source_to_project_reference.get(path).copied()
    }

    // projectreferencefilemapper.go:81
    pub(crate) fn get_project_reference_from_output_dts(&self, path: &Path) -> Option<P<SourceOutputAndProjectReference>> {
        self.output_dts_to_project_reference.get(path).copied()
    }

    // projectreferencefilemapper.go:85
    pub(crate) fn is_source_from_project_reference(&self, path: &Path) -> bool {
        self.use_source_of_project_reference && self.get_project_reference_from_source(path).is_some()
    }

    // projectreferencefilemapper.go:89
    pub(crate) fn get_compiler_options_for_file(&self, file_name: &str, path: &Path) -> P<CompilerOptions> {
        let redirect = self.get_redirect_parsed_command_line_for_resolution(file_name, path);
        module::get_compiler_options_with_redirect(self.config.compiler_options().unwrap(), redirect.map(as_resolved_project_reference))
    }

    // projectreferencefilemapper.go:94
    pub(crate) fn get_redirect_parsed_command_line_for_resolution(&self, _file_name: &str, path: &Path) -> Option<P<ParsedCommandLine>> {
        self.redirect_reference(path).map(|reference| reference.resolved)
    }

    // projectreferencefilemapper.go:99
    pub(crate) fn get_redirect_for_resolution(&self, file_name: &str, path: &Path) -> (Option<P<ParsedCommandLine>>, String) {
        match self.redirect_reference(path) {
            Some(reference) => (Some(reference.resolved), reference.source.clone()),
            None => (None, file_name.to_string()),
        }
    }

    // The reference `get_redirect_for_resolution` answers from, without building its file name string: the checker
    // asks for the compiler options of a file per import and per declaration (module format, emit syntax). A program
    // without references has empty source and output maps and never stores into `realpath_dts_to_source` (see
    // `projectReferenceFileMapperBuilder::has_no_references`), so it skips hashing the path and the shared map's lock.
    fn redirect_reference(&self, path: &Path) -> Option<P<SourceOutputAndProjectReference>> {
        if self.config.resolved_project_reference_paths().is_empty() {
            return None;
        }
        // Check if outputdts of source file from project reference
        if let Some(output) = self.get_project_reference_from_source(path) {
            return Some(output);
        }

        // Source file from project reference
        if let Some(result_from_dts) = self.get_project_reference_from_output_dts(path) {
            return Some(result_from_dts);
        }

        self.realpath_dts_to_source.load(path).flatten()
    }

    // projectreferencefilemapper.go:119
    pub(crate) fn get_resolved_reference_for(&self, path: &Path) -> (Option<P<ParsedCommandLine>>, bool) {
        match self.config_to_project_reference.get(path) {
            Some(config) => (*config, true),
            None => (None, false),
        }
    }

    // projectreferencefilemapper.go:124
    pub(crate) fn range_resolved_project_reference(
        &self,
        mut f: impl FnMut(&Path, Option<P<ParsedCommandLine>>, P<ParsedCommandLine>, usize) -> bool,
    ) -> bool {
        if self.config.project_references().is_empty() {
            return false;
        }
        let mut seen_ref: FxHashSet<Path> = FxHashSet::with_capacity_and_hasher(self.references_in_config_file.len(), Default::default());
        let root_config_path = self.root_config_path();
        seen_ref.insert(root_config_path.clone());
        let refs = self.references_in_config_file.get(&root_config_path).map(|v| v.as_slice()).unwrap_or(&[]);
        self.range_resolved_reference_worker(refs, &mut f, self.config, &mut seen_ref)
    }

    // projectreferencefilemapper.go:137
    fn range_resolved_reference_worker(
        &self,
        references: &[Path],
        f: &mut impl FnMut(&Path, Option<P<ParsedCommandLine>>, P<ParsedCommandLine>, usize) -> bool,
        parent: P<ParsedCommandLine>,
        seen_ref: &mut FxHashSet<Path>,
    ) -> bool {
        for (index, path) in references.iter().enumerate() {
            if !seen_ref.insert(path.clone()) {
                continue;
            }
            let config = self.config_to_project_reference.get(path).copied().flatten();
            if !f(path, config, parent, index) {
                return false;
            }
            let child_refs = self.references_in_config_file.get(path).map(|v| v.as_slice()).unwrap_or(&[]);
            // A reference that did not resolve has no references of its own (Go recurses with a nil parent over
            // an empty list).
            if let Some(config) = config {
                if !self.range_resolved_reference_worker(child_refs, f, config, seen_ref) {
                    return false;
                }
            }
        }
        true
    }

    // projectreferencefilemapper.go:159
    pub(crate) fn range_resolved_project_reference_in_child_config(
        &self,
        child_config: Option<P<ParsedCommandLine>>,
        mut f: impl FnMut(&Path, Option<P<ParsedCommandLine>>, P<ParsedCommandLine>, usize) -> bool,
    ) -> bool {
        let Some(child_config) = child_config else {
            return false;
        };
        let Some(child_config_file) = child_config.config_file else {
            return false;
        };
        let mut seen_ref: FxHashSet<Path> = FxHashSet::with_capacity_and_hasher(self.references_in_config_file.len(), Default::default());
        seen_ref.insert(child_config_file.source_file.path().clone());
        let refs = self.references_in_config_file.get(child_config_file.source_file.path()).map(|v| v.as_slice()).unwrap_or(&[]);
        self.range_resolved_reference_worker(refs, &mut f, self.config, &mut seen_ref)
    }
}

impl projectReferenceFileMapperBuilder {
    pub(crate) fn take_mapper(&self) -> Arc<projectReferenceFileMapper> {
        Arc::clone(&self.mapper)
    }

    // projectreferencefilemapper.go:174
    pub(crate) fn get_parse_file_redirect(&self, file_name: &str, path: &Path) -> String {
        if self.has_no_references() {
            return String::new();
        }
        if self.mapper.use_source_of_project_reference && self.mapper.get_project_reference_from_output_dts(path).is_none() {
            self.resolve_symlink(file_name, path);
        }
        self.mapper.get_parse_file_redirect(file_name, path)
    }

    // projectreferencefilemapper.go:181
    pub(crate) fn get_redirect_for_resolution(&self, file_name: &str, path: &Path) -> (Option<P<ParsedCommandLine>>, String) {
        if self.has_no_references() {
            return (None, file_name.to_string());
        }
        if self.mapper.get_project_reference_from_source(path).is_none() && self.mapper.get_project_reference_from_output_dts(path).is_none() {
            self.resolve_symlink(file_name, path);
        }
        self.mapper.get_redirect_for_resolution(file_name, path)
    }

    // projectreferencefilemapper.go:188
    pub(crate) fn get_compiler_options_for_file(&self, file_name: &str, path: &Path) -> P<CompilerOptions> {
        let (redirect, _) = self.get_redirect_for_resolution(file_name, path);
        module::get_compiler_options_with_redirect(self.mapper.config.compiler_options().unwrap(), redirect.map(as_resolved_project_reference))
    }

    // projectreferencefilemapper.go:193
    pub(crate) fn get_redirect_parsed_command_line_for_resolution(&self, file_name: &str, path: &Path) -> Option<P<ParsedCommandLine>> {
        let (redirect, _) = self.get_redirect_for_resolution(file_name, path);
        redirect
    }

    // tsrs-only: a program without project references has empty source and output maps, and `resolve_symlink` stores
    // into `realpath_dts_to_source` only with references, so every redirect query misses. Answering that up front
    // skips hashing the file's path into three maps (one of them a shared, locked map) per file and per import.
    fn has_no_references(&self) -> bool {
        self.mapper.config.resolved_project_reference_paths().is_empty()
    }

    // projectreferencefilemapper.go:198
    fn resolve_symlink(&self, file_name: &str, path: &Path) {
        // If preserveSymlinks is true, module resolution wont jump the symlink
        // but the resolved real path may be the .d.ts from project reference
        // Note:: Currently we try the real path only if the
        // file is from node_modules to avoid having to run real path on all file paths
        if self.mapper.realpath_dts_to_source.load(path).is_some() {
            return;
        }
        if !self.mapper.config.resolved_project_reference_paths().is_empty()
            && self.mapper.config.compiler_options().unwrap().preserve_symlinks.is_true()
        {
            if !file_name.contains("/node_modules/") {
                self.mapper.realpath_dts_to_source.store(path.clone(), None);
            } else {
                let real_declaration_path = tspath::to_path(
                    &self.host.fs().realpath(file_name),
                    self.host.get_current_directory(),
                    self.host.fs().use_case_sensitive_file_names(),
                );
                if real_declaration_path == *path {
                    self.mapper.realpath_dts_to_source.store(path.clone(), None);
                } else {
                    self.mapper.realpath_dts_to_source.store(path.clone(), self.mapper.get_project_reference_from_output_dts(&real_declaration_path));
                }
            }
        }
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[test]
    fn cached_host_retains_redirects_without_retaining_mapper() {
        let config = P::new(tsrs_tsoptions::new_parsed_command_line(tsrs_core::empty_compiler_options(), Vec::new(), Vec::new(), Default::default()));
        let mut data = projectReferenceRedirects::new(config, true);
        let source = P::new(SourceOutputAndProjectReference { source: "/src/ref.ts".into(), output_dts: "/out/ref.d.ts".into(), resolved: config });
        data.output_dts_to_project_reference.insert(Path::from("/out/ref.d.ts"), source);
        let mapper = Arc::new(projectReferenceFileMapper::new(data));
        let mapper_weak = Arc::downgrade(&mapper);
        let data_weak = Arc::downgrade(&mapper.data);
        let compiler_host = crate::new_compiler_host("/", Arc::new(tsrs_vfs::vfstest::from_map([("/src/ref.ts", "export {}; ")], true)), "", None, None);
        let compiler_host_weak = Arc::downgrade(&compiler_host);
        let host = mapper.resolution_host(resolution_host_for(compiler_host));
        let host_weak = Arc::downgrade(&host);
        let other_host = crate::new_compiler_host("/", Arc::new(tsrs_vfs::vfstest::from_map(Vec::<(&str, &str)>::new(), true)), "", None, None);
        assert!(Arc::ptr_eq(&host, &mapper.resolution_host(resolution_host_for(other_host))));

        drop(mapper);
        assert!(mapper_weak.upgrade().is_none());
        assert!(data_weak.upgrade().is_some());
        assert!(host.fs().file_exists("/out/ref.d.ts"));
        drop(host);
        assert!(host_weak.upgrade().is_none());
        assert!(data_weak.upgrade().is_none());
        assert!(compiler_host_weak.upgrade().is_none());
    }
}
