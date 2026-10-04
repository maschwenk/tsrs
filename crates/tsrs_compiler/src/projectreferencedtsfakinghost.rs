use std::time::SystemTime;

use tsrs_core::tspath::{self, Path};
use tsrs_core::{Tristate, P};
use tsrs_module::symlinks::{KnownDirectoryLink, KnownSymlinks};
use tsrs_module::{self as module, ResolutionHost};
use tsrs_vfs::{cachedvfs, Entries, FileInfo, FS};

use crate::projectreferencefilemapper::projectReferenceFileMapper;

struct projectReferenceDtsFakingHost {
    current_directory: String,
    fs: cachedvfs::FS<projectReferenceDtsFakingVfs>,
}

// projectreferencedtsfakinghost.go:22
pub(crate) fn new_project_reference_dts_faking_host(
    host: &'static dyn ResolutionHost,
    references: &'static projectReferenceFileMapper,
) -> &'static dyn ResolutionHost {
    // Create a new host that will fake the dts files
    Box::leak(Box::new(projectReferenceDtsFakingHost {
        current_directory: host.get_current_directory().to_string(),
        fs: cachedvfs::from(projectReferenceDtsFakingVfs {
            host,
            project_reference_file_mapper: references,
            known_symlinks: KnownSymlinks::default(),
        }),
    }))
}

impl ResolutionHost for projectReferenceDtsFakingHost {
    // projectreferencedtsfakinghost.go:35
    fn fs(&self) -> &dyn FS {
        &self.fs
    }

    // projectreferencedtsfakinghost.go:40
    fn get_current_directory(&self) -> &str {
        &self.current_directory
    }
}

struct projectReferenceDtsFakingVfs {
    host: &'static dyn ResolutionHost,
    project_reference_file_mapper: &'static projectReferenceFileMapper,
    known_symlinks: KnownSymlinks,
}

impl FS for projectReferenceDtsFakingVfs {
    // projectreferencedtsfakinghost.go:53
    fn use_case_sensitive_file_names(&self) -> bool {
        self.host.fs().use_case_sensitive_file_names()
    }

    // projectreferencedtsfakinghost.go:58
    fn file_exists(&self, path: &str) -> bool {
        if self.host.fs().file_exists(path) {
            return true;
        }
        if !tspath::is_declaration_file_name(path) {
            return false;
        }
        // Project references go to source file instead of .d.ts file
        self.file_or_directory_exists_using_source(path, true /*isFile*/)
    }

    // projectreferencedtsfakinghost.go:70
    fn read_file(&self, path: &str) -> Option<String> {
        // Dont need to override as we cannot mimick read file
        self.host.fs().read_file(path)
    }

    // projectreferencedtsfakinghost.go:76
    fn write_file(&self, _path: &str, _data: &str) -> Result<(), String> {
        panic!("should not be called by resolver")
    }

    // projectreferencedtsfakinghost.go:81
    fn append_file(&self, _path: &str, _data: &str) -> Result<(), String> {
        panic!("should not be called by resolver")
    }

    // projectreferencedtsfakinghost.go:86
    fn remove(&self, _path: &str) -> Result<(), String> {
        panic!("should not be called by resolver")
    }

    // projectreferencedtsfakinghost.go:91
    fn chtimes(&self, _path: &str, _a_time: SystemTime, _m_time: SystemTime) -> Result<(), String> {
        panic!("should not be called by resolver")
    }

    // projectreferencedtsfakinghost.go:96
    fn directory_exists(&self, path: &str) -> bool {
        if self.host.fs().directory_exists(path) {
            self.handle_directory_could_be_symlink(path);
            return true;
        }
        self.file_or_directory_exists_using_source(path, false /*isFile*/)
    }

    // projectreferencedtsfakinghost.go:105
    fn get_accessible_entries(&self, _path: &str) -> Entries {
        panic!("should not be called by resolver")
    }

    // projectreferencedtsfakinghost.go:110
    fn stat(&self, _path: &str) -> Option<FileInfo> {
        panic!("should not be called by resolver")
    }

    // projectreferencedtsfakinghost.go:115
    fn realpath(&self, path: &str) -> String {
        if let Some(result) = self.known_symlinks.files().load(&self.to_path(path)) {
            return result;
        }
        self.host.fs().realpath(path)
    }
}

impl projectReferenceDtsFakingVfs {
    // projectreferencedtsfakinghost.go:123
    fn to_path(&self, path: &str) -> Path {
        tspath::to_path(path, self.host.get_current_directory(), self.use_case_sensitive_file_names())
    }

    // projectreferencedtsfakinghost.go:127
    fn handle_directory_could_be_symlink(&self, directory: &str) {
        if tspath::contains_ignored_path(directory) {
            return;
        }

        // Because we already watch node_modules, handle symlinks in there
        if !directory.contains("/node_modules/") {
            return;
        }

        let directory_path = Path::new(tspath::ensure_trailing_directory_separator(self.to_path(directory).as_str()));
        if self.known_symlinks.directories().load(&directory_path).is_some() {
            return;
        }

        let real_directory = self.realpath(directory);
        if real_directory == directory {
            // not symlinked
            return;
        }
        let real_path = Path::new(tspath::ensure_trailing_directory_separator(self.to_path(&real_directory).as_str()));
        if real_path == directory_path {
            // not symlinked
            return;
        }
        self.known_symlinks.set_directory(
            directory,
            directory_path,
            Some(P::new(KnownDirectoryLink { real: tspath::ensure_trailing_directory_separator(&real_directory), real_path })),
        );
    }

    fn exists_using_source(&self, file_or_directory: &str, is_file: bool) -> Tristate {
        if is_file {
            self.file_exists_if_project_reference_dts(file_or_directory)
        } else {
            self.directory_exists_if_project_reference_decl_dir(file_or_directory)
        }
    }

    // projectreferencedtsfakinghost.go:160
    fn file_or_directory_exists_using_source(&self, file_or_directory: &str, is_file: bool) -> bool {
        // Check current directory or file
        let result = self.exists_using_source(file_or_directory, is_file);
        if result != Tristate::Unknown {
            return result == Tristate::True;
        }

        let file_or_directory_path = self.to_path(file_or_directory);
        if !file_or_directory_path.as_str().contains("/node_modules/") {
            return false;
        }
        // Check if the directory or file is a symlinked package
        let package_root = module::parse_node_module_from_path(file_or_directory, true /*isFolder*/);
        if !package_root.is_empty() {
            self.handle_directory_could_be_symlink(&package_root);
        }
        let known_directory_links = self.known_symlinks.directories();
        if known_directory_links.size() == 0 {
            return false;
        }
        if is_file && self.known_symlinks.files().load(&file_or_directory_path).is_some() {
            return true;
        }

        // If it contains node_modules check if its one of the symlinked path we know of
        let mut exists = false;
        known_directory_links.range(|directory_path, known_directory_link| {
            let Some(relative) = file_or_directory_path.as_str().strip_prefix(directory_path.as_str()) else {
                return true;
            };
            let Some(known_directory_link) = known_directory_link else {
                return true;
            };
            exists = self.exists_using_source(&format!("{}{}", known_directory_link.real_path.as_str(), relative), is_file).is_true();
            if exists {
                if is_file {
                    // Store the real path for the file
                    let absolute_path = tspath::get_normalized_absolute_path(file_or_directory, self.host.get_current_directory());
                    self.known_symlinks.set_file(
                        &absolute_path,
                        file_or_directory_path.clone(),
                        &format!("{}{}", known_directory_link.real, &absolute_path[directory_path.as_str().len()..]),
                    );
                }
                return false;
            }
            true
        });
        exists
    }

    // projectreferencedtsfakinghost.go:209
    fn file_exists_if_project_reference_dts(&self, file: &str) -> Tristate {
        if let Some(source) = self.project_reference_file_mapper.get_project_reference_from_output_dts(&self.to_path(file)) {
            return if self.host.fs().file_exists(&source.source) { Tristate::True } else { Tristate::False };
        }
        Tristate::Unknown
    }

    // projectreferencedtsfakinghost.go:217
    fn directory_exists_if_project_reference_decl_dir(&self, dir: &str) -> Tristate {
        let dir_path = self.to_path(dir);
        #[expect(clippy::iter_over_hash_type, reason = "returns True if any entry matches; the order does not matter")]
        for decl_dir_path in &self.project_reference_file_mapper.dts_directories {
            if dir_path.contains_path(decl_dir_path) || decl_dir_path.contains_path(&dir_path) {
                return Tristate::True;
            }
        }
        Tristate::Unknown
    }
}
