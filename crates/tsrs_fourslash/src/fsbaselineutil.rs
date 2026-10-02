// testutil/fsbaselineutil/differ.go: baselines the test map FS and its changes between calls. Only what
// statebaseline.go uses (`DefaultLibs` is never set by fourslash, so the lib branches never fire; kept as Go).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tsrs_vfs::iovfs::IoVFS;
use tsrs_vfs::vfstest::MapFS;
use tsrs_vfs::FileMode;

#[derive(Clone, Default)]
pub struct DiffEntry {
    pub content: String,
    pub m_time: Option<SystemTime>,
    pub is_written: bool,
    pub symlink_target: String,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub snap: HashMap<String, DiffEntry>,
    pub default_libs: HashSet<String>,
}

pub struct FSDiffer {
    pub fs: Arc<IoVFS<MapFS>>,
    pub default_libs: Option<Box<dyn Fn() -> Option<HashSet<String>> + Send + Sync>>,
    pub written_files: Mutex<HashSet<String>>,

    serialized_diff: Option<Snapshot>,
}

impl FSDiffer {
    pub fn new(fs: Arc<IoVFS<MapFS>>) -> FSDiffer {
        FSDiffer { fs, default_libs: None, written_files: Mutex::new(HashSet::new()), serialized_diff: None }
    }

    // differ.go:38
    pub fn map_fs(&self) -> &MapFS {
        self.fs.fsys()
    }

    // differ.go:42
    pub fn serialized_diff(&self) -> Option<&Snapshot> {
        self.serialized_diff.as_ref()
    }

    fn default_libs(&self) -> Option<HashSet<String>> {
        self.default_libs.as_ref().and_then(|f| f())
    }

    // differ.go:46
    pub fn baseline_fs_with_diff(&mut self, baseline: &mut String) {
        // todo: baselines the entire fs, possibly doesn't correctly diff all cases of emitted files, since emit isn't fully implemented and doesn't always emit the same way as strada
        let mut snap: HashMap<String, DiffEntry> = HashMap::new();

        let mut diffs: BTreeMap<String, String> = BTreeMap::new();

        let written = self.written_files.lock().unwrap().clone();
        for (path, file) in self.map_fs().entries() {
            if file.mode.intersects(FileMode::Symlink) {
                let Some(target) = self.map_fs().get_target_of_symlink(&path) else {
                    panic!("Failed to resolve symlink target: {}", path);
                };
                let new_entry = DiffEntry { symlink_target: target, ..Default::default() };
                self.add_fs_entry_diff(&mut diffs, Some(&new_entry), &path);
                snap.insert(path, new_entry);
                continue;
            } else if file.mode.is_regular() {
                let content = sanitize_internal_symbol_name(&String::from_utf8_lossy(&file.data));
                let new_entry = DiffEntry { content, m_time: file.mod_time, is_written: written.contains(&path), ..Default::default() };
                self.add_fs_entry_diff(&mut diffs, Some(&new_entry), &path);
                snap.insert(path, new_entry);
            }
        }
        if let Some(serialized) = &self.serialized_diff {
            let deleted: Vec<String> = serialized.snap.keys().filter(|p| self.map_fs().get_file_info(p).is_none()).cloned().collect();
            for path in deleted {
                // report deleted
                self.add_fs_entry_diff(&mut diffs, None, &path);
            }
        }
        let default_libs = self.default_libs().unwrap_or_default();
        self.serialized_diff = Some(Snapshot { snap, default_libs });
        for (path, diff) in &diffs {
            baseline.push_str(&format!("//// [{}] {}\n", path, diff));
        }
        baseline.push('\n');
        self.written_files.lock().unwrap().clear(); // Reset written files after baseline
    }

    // differ.go:117
    fn add_fs_entry_diff(&self, diffs: &mut BTreeMap<String, String>, new_dir_content: Option<&DiffEntry>, path: &str) {
        let (old_dir_content, default_libs) = match &self.serialized_diff {
            Some(s) => (s.snap.get(path), Some(&s.default_libs)),
            None => (None, None),
        };
        let current_libs = self.default_libs();
        // todo handle more cases of fs changes
        match (old_dir_content, new_dir_content) {
            (None, Some(new)) => {
                if current_libs.as_ref().is_none_or(|libs| !libs.contains(path)) {
                    if !new.symlink_target.is_empty() {
                        diffs.insert(path.to_string(), format!("-> {} *new*", new.symlink_target));
                    } else {
                        diffs.insert(path.to_string(), format!("*new* \n{}", new.content));
                    }
                }
            }
            (Some(_), None) => {
                diffs.insert(path.to_string(), "*deleted*".to_string());
            }
            (Some(old), Some(new)) => {
                if new.content != old.content {
                    diffs.insert(path.to_string(), format!("*modified* \n{}", new.content));
                } else if new.is_written {
                    diffs.insert(path.to_string(), "*rewrite with same content*".to_string());
                } else if new.m_time != old.m_time {
                    diffs.insert(path.to_string(), "*mTime changed*".to_string());
                } else if default_libs.is_some_and(|l| l.contains(path)) && current_libs.as_ref().is_some_and(|libs| !libs.contains(path)) {
                    // Lib file that was read
                    diffs.insert(path.to_string(), format!("*Lib*\n{}", new.content));
                }
            }
            (None, None) => {}
        }
    }
}

// differ.go:105
// Replaces internal symbol names of shape �@symbolName@123 with �@symbolName@<symbolId>
// to avoid baselining differences in symbol ids, which can change between runs.
pub fn sanitize_internal_symbol_name(s: &str) -> String {
    const MARK: &str = "\u{FFFD}@";
    if !s.contains(MARK) {
        return s.to_string();
    }
    // regexp `\x{FFFD}@[^@]+@[0-9]+`
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(MARK) {
        out.push_str(&rest[..i]);
        let after = &rest[i + MARK.len()..];
        let name_len = after.find('@').filter(|&n| n > 0);
        let digits = name_len.map(|n| after[n + 1..].bytes().take_while(|b| b.is_ascii_digit()).count()).unwrap_or(0);
        match name_len {
            Some(n) if digits > 0 => {
                out.push_str(MARK);
                out.push_str(&after[..n]);
                out.push_str("@<symbolId>");
                rest = &after[n + 1 + digits..];
            }
            _ => {
                out.push_str(MARK);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}
