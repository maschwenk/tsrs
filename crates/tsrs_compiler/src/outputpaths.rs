// The subset of Go's `outputpaths` package that type checking needs (common source directory
// computation used by option verification and sourceFileMayBeEmitted). Emit paths are not ported.

use tsrs_core::tspath;
use tsrs_core::CompilerOptions;

fn compute_common_source_directory_of_filenames(file_names: &[String], current_directory: &str, use_case_sensitive_file_names: bool) -> String {
    let mut common_path_components: Option<Vec<String>> = None;
    for source_file in file_names {
        // Each file contributes into common source file path
        let mut source_path_components = tspath::get_normalized_path_components(source_file, current_directory);

        // The base file name is not part of the common directory path
        source_path_components.pop();

        let Some(common) = &mut common_path_components else {
            // first file
            common_path_components = Some(source_path_components);
            continue;
        };

        let n = common.len().min(source_path_components.len());
        for i in 0..n {
            if tspath::get_canonical_file_name(&common[i], use_case_sensitive_file_names)
                != tspath::get_canonical_file_name(&source_path_components[i], use_case_sensitive_file_names)
            {
                if i == 0 {
                    // Failed to find any common path component
                    return String::new();
                }

                // New common path found that is 0 -> i-1
                common.truncate(i);
                break;
            }
        }

        // If the sourcePathComponents was shorter than the commonPathComponents, truncate to the sourcePathComponents
        if source_path_components.len() < common.len() {
            common.truncate(source_path_components.len());
        }
    }

    match common_path_components {
        Some(common) if !common.is_empty() => tspath::get_path_from_path_components(&common),
        // Can happen when all input files are .d.ts files
        _ => current_directory.to_string(),
    }
}

pub fn get_computed_common_source_directory(emitted_files: &[String], current_directory: &str, use_case_sensitive_file_names: bool) -> String {
    let mut common_source_directory =
        compute_common_source_directory_of_filenames(emitted_files, current_directory, use_case_sensitive_file_names);
    if !common_source_directory.is_empty() {
        common_source_directory = tspath::ensure_trailing_directory_separator(&common_source_directory);
    }
    common_source_directory
}

pub fn get_common_source_directory(
    options: &CompilerOptions,
    files: impl FnOnce() -> Vec<String>,
    current_directory: &str,
    use_case_sensitive_file_names: bool,
    check_source_files_belong_to_path: Option<&dyn Fn(&[String], &str) -> bool>,
) -> String {
    let mut common_source_directory;
    if !options.root_dir.is_empty() {
        // If a rootDir is specified use it as the commonSourceDirectory
        common_source_directory = options.root_dir.clone();
        if let Some(check) = check_source_files_belong_to_path {
            check(&files(), &options.root_dir);
        }
    } else if !options.config_file_path.is_empty() {
        // If the rootDir is not specified, then the common source directory is the directory of the config file.
        common_source_directory = tspath::get_directory_path(&options.config_file_path);
        if let Some(check) = check_source_files_belong_to_path {
            check(&files(), &common_source_directory);
        }
    } else {
        common_source_directory = compute_common_source_directory_of_filenames(&files(), current_directory, use_case_sensitive_file_names);
    }

    if !common_source_directory.is_empty() {
        // Make sure directory path ends with directory separator so this string can directly
        // used to replace with "" to get the relative path of the source file and the relative path doesn't
        // start with / making it rooted path
        common_source_directory = tspath::ensure_trailing_directory_separator(&common_source_directory);
    }

    common_source_directory
}

pub fn get_source_file_path_in_new_dir_worker(
    file_name: &str,
    new_dir_path: &str,
    current_directory: &str,
    common_source_directory: &str,
    use_case_sensitive_file_names: bool,
) -> String {
    let mut source_file_path = tspath::get_normalized_absolute_path(file_name, current_directory);
    if let Some(trimmed) = tspath::trim_file_path_prefix(&source_file_path, common_source_directory, use_case_sensitive_file_names) {
        source_file_path = trimmed.to_string();
    }
    tspath::combine_paths(new_dir_path, &[&source_file_path])
}
