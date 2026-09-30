// Go package `tsoptions/tsoptionstest`: test helpers shared with other crates' tests.

use tsrs_core::tspath;
use tsrs_core::P;
use tsrs_vfs::vfstest;
use tsrs_vfs::FS;

use crate::parsedcommandline::ParsedCommandLine;
use crate::tsconfigparsing::{new_tsconfig_source_file_from_file_path, parse_json_source_file_config_file_content, ParseConfigHost};

pub fn get_parsed_command_line(
    json_text: &str,
    files: &[(&str, &str)],
    current_directory: &str,
    use_case_sensitive_file_names: bool,
) -> ParsedCommandLine {
    let host = new_vfs_parse_config_host(files, current_directory, use_case_sensitive_file_names);
    let config_file_name = tspath::combine_paths(current_directory, &["tsconfig.json"]);
    let tsconfig_source_file = new_tsconfig_source_file_from_file_path(
        &config_file_name,
        tspath::to_path(&config_file_name, current_directory, use_case_sensitive_file_names),
        json_text,
    );
    parse_json_source_file_config_file_content(tsconfig_source_file, host, current_directory, None, None, &config_file_name, &[], None)
}

fn fix_root(path: &str) -> &str {
    let root_length = tspath::get_root_length(path);
    if root_length == 0 {
        return path;
    }
    if path.len() == root_length {
        return ".";
    }
    &path[root_length..]
}

pub struct VfsParseConfigHost {
    pub vfs: Box<dyn FS>,
    pub current_directory: String,
}

impl ParseConfigHost for VfsParseConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.vfs
    }

    fn get_current_directory(&self) -> &str {
        &self.current_directory
    }
}

// Hosts are leaked: config parsing keeps `&'static dyn ParseConfigHost` (the module resolver requires it).
pub fn new_vfs_parse_config_host(
    files: &[(&str, &str)],
    current_directory: &str,
    use_case_sensitive_file_names: bool,
) -> &'static VfsParseConfigHost {
    P::new(VfsParseConfigHost {
        vfs: Box::new(vfstest::from_map(files.iter().map(|(k, v)| (*k, v.to_string())), use_case_sensitive_file_names)),
        current_directory: current_directory.to_string(),
    })
    .get()
}
