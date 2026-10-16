use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rustc_hash::FxHashMap;
use tsrs_vfs::{Entries, FS, FileInfo};

// Record responses, not just successful file reads: config globs and module resolution also need
// directory entries, failed probes, and the OS's symlink and case-sensitivity decisions.
#[derive(Clone, Default)]
pub(crate) struct SnapshotFs {
    case_sensitive: bool,
    files: FxHashMap<String, Option<String>>,
    file_exists: FxHashMap<String, bool>,
    directory_exists: FxHashMap<String, bool>,
    entries: FxHashMap<String, Entries>,
    metadata: FxHashMap<String, Option<FileInfo>>,
    realpaths: FxHashMap<String, String>,
}

pub(crate) struct RecordingFs {
    inner: Arc<dyn FS>,
    snapshot: Mutex<SnapshotFs>,
}

impl RecordingFs {
    pub(crate) fn new(inner: Arc<dyn FS>) -> Self {
        let case_sensitive = inner.use_case_sensitive_file_names();
        Self {
            inner,
            snapshot: Mutex::new(SnapshotFs {
                case_sensitive,
                ..SnapshotFs::default()
            }),
        }
    }

    pub(crate) fn freeze(&self) -> SnapshotFs {
        self.snapshot.lock().unwrap().clone()
    }
}

macro_rules! record_read {
    ($method:ident, $field:ident, $result:ty) => {
        fn $method(&self, path: &str) -> $result {
            let value = self.inner.$method(path);
            let mut snapshot = self.snapshot.lock().unwrap();
            let recorded = snapshot
                .$field
                .entry(path.to_string())
                .or_insert_with(|| value.clone());
            assert_eq!(
                recorded,
                &value,
                "filesystem changed during capture: {}({path:?})",
                stringify!($method)
            );
            value
        }
    };
}

macro_rules! replay_read {
    ($method:ident, $field:ident, $result:ty) => {
        fn $method(&self, path: &str) -> $result {
            self.$field
                .get(path)
                .unwrap_or_else(|| {
                    panic!(
                        "unrecorded filesystem request: {}({path:?})",
                        stringify!($method)
                    )
                })
                .clone()
        }
    };
}

macro_rules! reject_mutations {
    () => {
        fn write_file(&self, path: &str, _data: &str) -> Result<(), String> {
            panic!("benchmark filesystem mutation: write_file({path:?})")
        }

        fn append_file(&self, path: &str, _data: &str) -> Result<(), String> {
            panic!("benchmark filesystem mutation: append_file({path:?})")
        }

        fn remove(&self, path: &str) -> Result<(), String> {
            panic!("benchmark filesystem mutation: remove({path:?})")
        }

        fn chtimes(
            &self,
            path: &str,
            _a_time: SystemTime,
            _m_time: SystemTime,
        ) -> Result<(), String> {
            panic!("benchmark filesystem mutation: chtimes({path:?})")
        }
    };
}

impl FS for RecordingFs {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.snapshot.lock().unwrap().case_sensitive
    }

    record_read!(read_file, files, Option<String>);
    record_read!(file_exists, file_exists, bool);
    record_read!(directory_exists, directory_exists, bool);
    record_read!(get_accessible_entries, entries, Entries);
    record_read!(stat, metadata, Option<FileInfo>);
    record_read!(realpath, realpaths, String);
    reject_mutations!();
}

// Deliberately has no OS filesystem handle. Even a new compiler probe must have been seen during
// capture; treating an unrecorded path as absent would silently change the program being measured.
impl FS for SnapshotFs {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.case_sensitive
    }

    replay_read!(read_file, files, Option<String>);
    replay_read!(file_exists, file_exists, bool);
    replay_read!(directory_exists, directory_exists, bool);
    replay_read!(get_accessible_entries, entries, Entries);
    replay_read!(stat, metadata, Option<FileInfo>);
    replay_read!(realpath, realpaths, String);
    reject_mutations!();
}
