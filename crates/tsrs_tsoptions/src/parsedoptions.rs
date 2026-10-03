use tsrs_core::{CompilerOptions, ProjectReference, TypeAcquisition, WatchOptions, P};

use crate::contentmappers::Mapper;

#[derive(Clone, Debug, Default)]
pub struct ParsedOptions {
    pub compiler_options: Option<P<CompilerOptions>>,
    pub watch_options: Option<WatchOptions>,
    pub type_acquisition: Option<TypeAcquisition>,

    pub file_names: Vec<String>,
    pub project_references: Vec<ProjectReference>,
    // Go keeps `ProjectReferences` nil when the config has no "references" (build mode treats files: [] plus
    // references: [] as a solution).
    pub project_references_is_nil: bool,
    pub content_mappers: Vec<Mapper>,
}
