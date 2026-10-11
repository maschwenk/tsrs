// tsrs-only: each resolver call keeps its original FS dispatch and result. The VFS profiling
// feature supplies the counters; its disabled stubs inline away, including the result labels.
use std::time::SystemTime;

use tsrs_vfs::fsstats::{enabled, logical};
use tsrs_vfs::{Entries, FS, FileInfo};

pub(crate) struct ObservedFS<'a> {
    fs: &'a dyn FS,
    site: &'static str,
}

impl<'a> ObservedFS<'a> {
    #[inline]
    pub(crate) fn new(fs: &'a dyn FS, site: &'static str) -> Self {
        Self { fs, site }
    }

    // Directory matching takes a trait object. Keep its original dispatch when profiling is off.
    #[inline]
    pub(crate) fn as_fs(&self) -> &dyn FS {
        if enabled() { self } else { self.fs }
    }
}

#[inline]
#[expect(clippy::trivially_copy_pass_by_ref, reason = "logical result callbacks borrow the value that is returned unchanged")]
fn boolean_result(value: &bool) -> &'static str {
    if *value { "true" } else { "false" }
}

#[inline]
fn option_result<T>(value: &Option<T>) -> &'static str {
    if value.is_some() { "some" } else { "none" }
}

#[inline]
fn write_result(value: &Result<(), String>) -> &'static str {
    if value.is_ok() { "ok" } else { "error" }
}

impl FS for ObservedFS<'_> {
    #[inline]
    fn use_case_sensitive_file_names(&self) -> bool {
        logical(
            self.site,
            "use_case_sensitive_file_names",
            "",
            || self.fs.use_case_sensitive_file_names(),
            boolean_result,
        )
    }

    #[inline]
    fn file_exists(&self, path: &str) -> bool {
        logical(
            self.site,
            "file_exists",
            path,
            || self.fs.file_exists(path),
            boolean_result,
        )
    }

    #[inline]
    fn read_file(&self, path: &str) -> Option<String> {
        logical(
            self.site,
            "read_file",
            path,
            || self.fs.read_file(path),
            option_result,
        )
    }

    #[inline]
    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        logical(
            self.site,
            "write_file",
            path,
            || self.fs.write_file(path, data),
            write_result,
        )
    }

    #[inline]
    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        logical(
            self.site,
            "append_file",
            path,
            || self.fs.append_file(path, data),
            write_result,
        )
    }

    #[inline]
    fn remove(&self, path: &str) -> Result<(), String> {
        logical(
            self.site,
            "remove",
            path,
            || self.fs.remove(path),
            write_result,
        )
    }

    #[inline]
    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        logical(
            self.site,
            "chtimes",
            path,
            || self.fs.chtimes(path, a_time, m_time),
            write_result,
        )
    }

    #[inline]
    fn directory_exists(&self, path: &str) -> bool {
        logical(
            self.site,
            "directory_exists",
            path,
            || self.fs.directory_exists(path),
            boolean_result,
        )
    }

    #[inline]
    fn get_accessible_entries(&self, path: &str) -> Entries {
        logical(
            self.site,
            "get_accessible_entries",
            path,
            || self.fs.get_accessible_entries(path),
            |entries| {
                if entries.files.is_empty() && entries.directories.is_empty() {
                    "empty"
                } else {
                    "nonempty"
                }
            },
        )
    }

    #[inline]
    fn stat(&self, path: &str) -> Option<FileInfo> {
        logical(
            self.site,
            "stat",
            path,
            || self.fs.stat(path),
            option_result,
        )
    }

    #[inline]
    fn realpath(&self, path: &str) -> String {
        logical(
            self.site,
            "realpath",
            path,
            || self.fs.realpath(path),
            |result| {
                // The FS API also returns the input when canonicalization fails.
                if result == path {
                    "identity"
                } else {
                    "changed"
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::UNIX_EPOCH;

    use super::*;

    const RAW_PATH: &str = "/raw\\dir/../file.ts";

    #[derive(Default)]
    struct RecordingFS {
        calls: Mutex<Vec<&'static str>>,
    }

    impl RecordingFS {
        fn record(&self, operation: &'static str, path: &str) {
            assert_eq!(path, RAW_PATH);
            self.calls.lock().unwrap().push(operation);
        }
    }

    impl FS for RecordingFS {
        fn use_case_sensitive_file_names(&self) -> bool {
            self.calls.lock().unwrap().push("case_sensitive");
            true
        }

        fn file_exists(&self, path: &str) -> bool {
            self.record("file_exists", path);
            true
        }

        fn read_file(&self, path: &str) -> Option<String> {
            self.record("read_file", path);
            None
        }

        fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
            self.record("write_file", path);
            assert_eq!(data, "written");
            Err("custom write error".to_string())
        }

        fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
            self.record("append_file", path);
            assert_eq!(data, "appended");
            Ok(())
        }

        fn remove(&self, path: &str) -> Result<(), String> {
            self.record("remove", path);
            Err("custom remove error".to_string())
        }

        fn chtimes(
            &self,
            path: &str,
            a_time: SystemTime,
            m_time: SystemTime,
        ) -> Result<(), String> {
            self.record("chtimes", path);
            assert_eq!(a_time, UNIX_EPOCH);
            assert_eq!(m_time, UNIX_EPOCH);
            Ok(())
        }

        fn directory_exists(&self, path: &str) -> bool {
            self.record("directory_exists", path);
            false
        }

        fn get_accessible_entries(&self, path: &str) -> Entries {
            self.record("get_accessible_entries", path);
            Entries {
                files: vec!["file.ts".to_string()],
                directories: vec!["dir".to_string()],
                symlinks: Some(Default::default()),
            }
        }

        fn stat(&self, path: &str) -> Option<FileInfo> {
            self.record("stat", path);
            None
        }

        fn realpath(&self, path: &str) -> String {
            self.record("realpath", path);
            path.to_string()
        }
    }

    #[test]
    fn observing_preserves_custom_fs_calls_and_results() {
        let fs = RecordingFS::default();
        let observed = ObservedFS::new(&fs, "test.custom_fs");
        assert!(observed.use_case_sensitive_file_names());
        assert!(observed.file_exists(RAW_PATH));
        assert_eq!(observed.read_file(RAW_PATH), None);
        assert_eq!(
            observed.write_file(RAW_PATH, "written"),
            Err("custom write error".to_string())
        );
        assert_eq!(observed.append_file(RAW_PATH, "appended"), Ok(()));
        assert_eq!(
            observed.remove(RAW_PATH),
            Err("custom remove error".to_string())
        );
        assert_eq!(observed.chtimes(RAW_PATH, UNIX_EPOCH, UNIX_EPOCH), Ok(()));
        assert!(!observed.directory_exists(RAW_PATH));
        assert_eq!(
            observed.get_accessible_entries(RAW_PATH),
            Entries {
                files: vec!["file.ts".to_string()],
                directories: vec!["dir".to_string()],
                symlinks: Some(Default::default()),
            }
        );
        assert_eq!(observed.stat(RAW_PATH), None);
        assert_eq!(observed.realpath(RAW_PATH), RAW_PATH);
        assert_eq!(
            *fs.calls.lock().unwrap(),
            [
                "case_sensitive",
                "file_exists",
                "read_file",
                "write_file",
                "append_file",
                "remove",
                "chtimes",
                "directory_exists",
                "get_accessible_entries",
                "stat",
                "realpath",
            ]
        );
    }
}
