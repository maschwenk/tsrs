use std::sync::Arc;

use tsrs_core::tspath::Path;
use tsrs_core::{CompilerOptions, P};
use tsrs_module::{self as module, ResolutionHost, ResolvedProjectReference};
use tsrs_tsoptions::ParsedCommandLine;
use tsrs_vfs::FS;

use crate::host::CompilerHost;
use crate::program::ProgramConfig;

// Go `tsoptions.SourceOutputAndProjectReference` (only reachable through project references).
pub struct SourceOutputAndProjectReference {
    pub source: String,
    pub output_dts: String,
    pub resolved: &'static dyn ResolvedProjectReference,
}

// Project references are not ported yet (low priority for type checking a single project).
// The mapper keeps Go's method surface but behaves exactly like Go's mapper for a config
// without references: no redirects, no source/output mappings.
pub(crate) struct projectReferenceFileMapper {
    pub(crate) config: P<ParsedCommandLine>,
    pub(crate) use_source_of_project_reference: bool,
}

pub(crate) struct projectReferenceFileMapperBuilder {
    mapper: Option<projectReferenceFileMapper>,
    pub(crate) host: &'static dyn ResolutionHost,
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

pub(crate) fn resolution_host_for(host: Arc<dyn CompilerHost>) -> &'static dyn ResolutionHost {
    Box::leak(Box::new(compilerResolutionHost(host)))
}

impl projectReferenceFileMapperBuilder {
    pub(crate) fn new(opts: &ProgramConfig, host: Arc<dyn CompilerHost>) -> projectReferenceFileMapperBuilder {
        projectReferenceFileMapperBuilder {
            mapper: Some(projectReferenceFileMapper {
                config: opts.config,
                use_source_of_project_reference: opts.can_use_project_reference_source(),
            }),
            host: resolution_host_for(host),
        }
    }

    pub(crate) fn take_mapper(&mut self) -> projectReferenceFileMapper {
        self.mapper.take().unwrap()
    }

    fn mapper(&self) -> &projectReferenceFileMapper {
        self.mapper.as_ref().unwrap()
    }

    pub(crate) fn get_parse_file_redirect(&self, file_name: &str, path: &Path) -> String {
        self.mapper().get_parse_file_redirect(file_name, path)
    }

    pub(crate) fn get_compiler_options_for_file(&self, file_name: &str, path: &Path) -> P<CompilerOptions> {
        self.mapper().get_compiler_options_for_file(file_name, path)
    }

    pub(crate) fn get_redirect_for_resolution(&self, file_name: &str, path: &Path) -> (Option<&'static dyn ResolvedProjectReference>, String) {
        self.mapper().get_redirect_for_resolution(file_name, path)
    }

    pub(crate) fn get_redirect_parsed_command_line_for_resolution(
        &self,
        file_name: &str,
        path: &Path,
    ) -> Option<&'static dyn ResolvedProjectReference> {
        self.mapper().get_redirect_parsed_command_line_for_resolution(file_name, path)
    }
}

impl projectReferenceFileMapper {
    pub(crate) fn resolution_host(&self, host: &'static dyn ResolutionHost) -> &'static dyn ResolutionHost {
        host
    }

    pub(crate) fn get_parse_file_redirect(&self, _file_name: &str, path: &Path) -> String {
        if self.use_source_of_project_reference {
            // Map to source file from project reference
            if let Some(source) = self.get_project_reference_from_output_dts(path) {
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

    pub(crate) fn get_resolved_project_references(&self) -> Vec<P<ParsedCommandLine>> {
        Vec::new()
    }

    pub(crate) fn get_project_reference_from_source(&self, _path: &Path) -> Option<&'static SourceOutputAndProjectReference> {
        None
    }

    pub(crate) fn get_project_reference_from_output_dts(&self, _path: &Path) -> Option<&'static SourceOutputAndProjectReference> {
        None
    }

    pub(crate) fn is_source_from_project_reference(&self, path: &Path) -> bool {
        self.use_source_of_project_reference && self.get_project_reference_from_source(path).is_some()
    }

    pub(crate) fn get_compiler_options_for_file(&self, file_name: &str, path: &Path) -> P<CompilerOptions> {
        let redirect = self.get_redirect_parsed_command_line_for_resolution(file_name, path);
        module::get_compiler_options_with_redirect(self.config.compiler_options().unwrap(), redirect)
    }

    pub(crate) fn get_redirect_parsed_command_line_for_resolution(
        &self,
        file_name: &str,
        path: &Path,
    ) -> Option<&'static dyn ResolvedProjectReference> {
        self.get_redirect_for_resolution(file_name, path).0
    }

    pub(crate) fn get_redirect_for_resolution(&self, file_name: &str, path: &Path) -> (Option<&'static dyn ResolvedProjectReference>, String) {
        // Check if outputdts of source file from project reference
        if let Some(output) = self.get_project_reference_from_source(path) {
            return (Some(output.resolved), output.source.clone());
        }

        // Source file from project reference
        if let Some(result_from_dts) = self.get_project_reference_from_output_dts(path) {
            return (Some(result_from_dts.resolved), result_from_dts.source.clone());
        }
        (None, file_name.to_string())
    }

    pub(crate) fn range_resolved_project_reference(
        &self,
        _f: impl FnMut(&Path, Option<P<ParsedCommandLine>>, P<ParsedCommandLine>, usize) -> bool,
    ) -> bool {
        false
    }
}
