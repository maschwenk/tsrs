use std::sync::Arc;

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
    // Go `[]*Mapper`: the copies of a config (`reload_file_names_of_parsed_command_line`) share the mappers. A content
    // mapper project knows the mappers it was made with by address (hostimpl.rs `mapper_key`).
    pub content_mappers: Arc<[Mapper]>,
}
