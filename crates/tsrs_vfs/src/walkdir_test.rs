use std::sync::Mutex;
use std::time::SystemTime;

use crate::vfstest;
use crate::{walk_dir, Entries, FileInfo, FileMode, FsError, WalkDirError, FS};

// A stand-in for wrapvfs.Wrap with Replacements: each replacement overrides one method.
#[derive(Default)]
struct Replacements {
    get_accessible_entries: Option<Box<dyn Fn(&dyn FS, &str) -> Entries + Send + Sync>>,
    realpath: Option<Box<dyn Fn(&dyn FS, &str) -> String + Send + Sync>>,
    stat: Option<Box<dyn Fn(&dyn FS, &str) -> Option<FileInfo> + Send + Sync>>,
}

struct Wrapped<T: FS> {
    fs: T,
    r: Replacements,
}

impl<T: FS> FS for Wrapped<T> {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.fs.use_case_sensitive_file_names()
    }
    fn file_exists(&self, path: &str) -> bool {
        self.fs.file_exists(path)
    }
    fn read_file(&self, path: &str) -> Option<String> {
        self.fs.read_file(path)
    }
    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.fs.write_file(path, data)
    }
    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.fs.append_file(path, data)
    }
    fn remove(&self, path: &str) -> Result<(), String> {
        self.fs.remove(path)
    }
    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        self.fs.chtimes(path, a_time, m_time)
    }
    fn directory_exists(&self, path: &str) -> bool {
        self.fs.directory_exists(path)
    }
    fn get_accessible_entries(&self, path: &str) -> Entries {
        match &self.r.get_accessible_entries {
            Some(f) => f(&self.fs, path),
            None => self.fs.get_accessible_entries(path),
        }
    }
    fn stat(&self, path: &str) -> Option<FileInfo> {
        match &self.r.stat {
            Some(f) => f(&self.fs, path),
            None => self.fs.stat(path),
        }
    }
    fn realpath(&self, path: &str) -> String {
        match &self.r.realpath {
            Some(f) => f(&self.fs, path),
            None => self.fs.realpath(path),
        }
    }
}

#[test]
fn test_walk_dir() {
    let base = vfstest::from_map([("/root/a.ts", ""), ("/root/dir/b.ts", ""), ("/root/link/hidden.ts", ""), ("/target/hidden.ts", "")], true);
    let file_system = Wrapped {
        fs: base,
        r: Replacements {
            get_accessible_entries: Some(Box::new(|base, path| {
                let mut entries = base.get_accessible_entries(path);
                entries.symlinks = None;
                if path == "/root" {
                    entries.files.push("C:/foreign.ts".to_string());
                }
                entries
            })),
            realpath: Some(Box::new(|_, path| match path {
                "/root/link" => "/target".to_string(),
                "/root/link/hidden.ts" => "/target/hidden.ts".to_string(),
                _ => path.to_string(),
            })),
            ..Default::default()
        },
    };

    let mut paths: Vec<String> = Vec::new();
    let mut modes: Vec<FileMode> = Vec::new();
    let err = walk_dir(&file_system, "/root", &mut |path, entry, err| {
        assert!(err.is_none());
        let entry = entry.unwrap();
        paths.push(path.to_string());
        modes.push(entry.type_());
        if entry.type_().intersects(FileMode::Symlink) {
            let info = entry.info(&file_system).unwrap();
            assert_eq!(info.mode(), FileMode::Symlink);
            assert!(!info.is_dir());
        }
        Ok(())
    });
    assert_eq!(err, Ok(()));
    assert_eq!(paths, vec!["/root", "/root/a.ts", "/root/dir", "/root/dir/b.ts", "/root/link"]);
    assert_eq!(modes, vec![FileMode::Dir, FileMode::None, FileMode::Dir, FileMode::None, FileMode::Symlink]);
}

#[test]
fn test_walk_dir_does_not_follow_root_symlink() {
    let base = vfstest::from_map([("/root/link/hidden.ts", ""), ("/target/hidden.ts", "")], true);
    let file_system = Wrapped {
        fs: base,
        r: Replacements {
            get_accessible_entries: Some(Box::new(|base, path| {
                let mut entries = base.get_accessible_entries(path);
                entries.symlinks = None;
                entries
            })),
            realpath: Some(Box::new(|_, path| if path == "/root/link" { "/target".to_string() } else { path.to_string() })),
            ..Default::default()
        },
    };

    let mut paths: Vec<String> = Vec::new();
    let err = walk_dir(&file_system, "/root/link", &mut |path, entry, err| {
        assert!(err.is_none());
        paths.push(path.to_string());
        assert_eq!(entry.unwrap().type_(), FileMode::Symlink);
        Ok(())
    });
    assert_eq!(err, Ok(()));
    assert_eq!(paths, vec!["/root/link"]);
}

#[test]
fn test_walk_dir_reports_root_file_symlink() {
    let base = vfstest::from_map([("/target/file.ts", "")], true);
    let file_system = Wrapped {
        fs: base,
        r: Replacements {
            stat: Some(Box::new(|base, path| if path == "/root/link.ts" { base.stat("/target/file.ts") } else { base.stat(path) })),
            realpath: Some(Box::new(|_, path| if path == "/root/link.ts" { "/target/file.ts".to_string() } else { path.to_string() })),
            ..Default::default()
        },
    };

    let err = walk_dir(&file_system, "/root/link.ts", &mut |path, entry, err| {
        assert!(err.is_none());
        let entry = entry.unwrap();
        assert_eq!(path, "/root/link.ts");
        assert_eq!(entry.name(), "link.ts");
        assert_eq!(entry.type_(), FileMode::Symlink);
        Ok(())
    });
    assert_eq!(err, Ok(()));
}

#[test]
fn test_walk_dir_skip_dir() {
    let file_system = vfstest::from_map([("/root/a/hidden.ts", ""), ("/root/b.ts", "")], true);
    let mut paths: Vec<String> = Vec::new();
    let err = walk_dir(&file_system, "/root", &mut |path, entry, err| {
        assert!(err.is_none());
        paths.push(path.to_string());
        if path == "/root/a" {
            return Err(WalkDirError::SkipDir);
        }
        Ok(())
    });
    assert_eq!(err, Ok(()));
    assert_eq!(paths, vec!["/root", "/root/a", "/root/b.ts"]);
}

#[test]
fn test_walk_dir_skip_all() {
    let file_system = vfstest::from_map([("/root/a.ts", ""), ("/root/b.ts", "")], true);
    let mut paths: Vec<String> = Vec::new();
    let err = walk_dir(&file_system, "/root", &mut |path, entry, err| {
        assert!(err.is_none());
        paths.push(path.to_string());
        if path == "/root/a.ts" {
            return Err(WalkDirError::SkipAll);
        }
        Ok(())
    });
    assert_eq!(err, Ok(()));
    assert_eq!(paths, vec!["/root", "/root/a.ts"]);
}

#[test]
fn test_walk_dir_consumes_skip_dir_for_root_file() {
    let file_system = vfstest::from_map([("/root.ts", "")], true);
    let err = walk_dir(&file_system, "/root.ts", &mut |path, entry, err| {
        assert!(err.is_none());
        Err(WalkDirError::SkipDir)
    });
    assert_eq!(err, Ok(()));
}

#[test]
fn test_walk_dir_consumes_skip_for_missing_root() {
    let file_system = vfstest::from_map(Vec::<(&str, &str)>::new(), true);
    for sentinel in [WalkDirError::SkipDir, WalkDirError::SkipAll] {
        let err = walk_dir(&file_system, "/missing", &mut |path, entry, err| {
            assert_eq!(err, Some(&FsError::NotExist));
            Err(sentinel.clone())
        });
        assert_eq!(err, Ok(()));
    }
}

#[test]
fn test_walk_dir_uses_symlink_metadata_without_realpath_calls() {
    static REALPATH_CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    let base = vfstest::from_map([("/root/dir/file.ts", "")], true);
    let file_system = Wrapped {
        fs: base,
        r: Replacements {
            realpath: Some(Box::new(|_, path| {
                REALPATH_CALLS.lock().unwrap().push(path.to_string());
                path.to_string()
            })),
            ..Default::default()
        },
    };

    let err = walk_dir(&file_system, "/root", &mut |path, entry, err| match err {
        Some(err) => Err(WalkDirError::Err(err.clone())),
        None => Ok(()),
    });
    assert_eq!(err, Ok(()));
    assert!(!REALPATH_CALLS.lock().unwrap().iter().any(|p| p == "/root/dir"));
}
