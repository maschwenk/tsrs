use std::sync::Mutex;
use std::time::SystemTime;

use super::*;
use crate::iovfs::IoVFS;
use crate::vfstest::{self, MapFS};

// A stand-in for vfsmock.FSMock: records calls and forwards to the wrapped FS.
#[derive(Default)]
struct Calls {
    directory_exists: Vec<String>,
    file_exists: Vec<String>,
    get_accessible_entries: Vec<String>,
    realpath: Vec<String>,
    stat: Vec<String>,
    read_file: Vec<String>,
    use_case_sensitive_file_names: usize,
    remove: Vec<String>,
    write_file: Vec<(String, String)>,
}

struct FSMock {
    fs: IoVFS<MapFS>,
    calls: Mutex<Calls>,
}

impl VFS for &FSMock {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.calls.lock().unwrap().use_case_sensitive_file_names += 1;
        self.fs.use_case_sensitive_file_names()
    }
    fn file_exists(&self, path: &str) -> bool {
        self.calls.lock().unwrap().file_exists.push(path.to_string());
        self.fs.file_exists(path)
    }
    fn read_file(&self, path: &str) -> Option<String> {
        self.calls.lock().unwrap().read_file.push(path.to_string());
        self.fs.read_file(path)
    }
    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.calls.lock().unwrap().write_file.push((path.to_string(), data.to_string()));
        self.fs.write_file(path, data)
    }
    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.fs.append_file(path, data)
    }
    fn remove(&self, path: &str) -> Result<(), String> {
        self.calls.lock().unwrap().remove.push(path.to_string());
        self.fs.remove(path)
    }
    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        self.fs.chtimes(path, a_time, m_time)
    }
    fn directory_exists(&self, path: &str) -> bool {
        self.calls.lock().unwrap().directory_exists.push(path.to_string());
        self.fs.directory_exists(path)
    }
    fn get_accessible_entries(&self, path: &str) -> Entries {
        self.calls.lock().unwrap().get_accessible_entries.push(path.to_string());
        self.fs.get_accessible_entries(path)
    }
    fn stat(&self, path: &str) -> Option<FileInfo> {
        self.calls.lock().unwrap().stat.push(path.to_string());
        self.fs.stat(path)
    }
    fn realpath(&self, path: &str) -> String {
        self.calls.lock().unwrap().realpath.push(path.to_string());
        self.fs.realpath(path)
    }
}

fn create_mock_fs() -> FSMock {
    FSMock {
        fs: vfstest::from_map([("/some/path/file.txt", "hello world")], true),
        calls: Mutex::new(Calls::default()),
    }
}

// Runs the call sequence shared by the cached-method tests and checks the
// underlying call count after each step.
fn check_cached(call: impl Fn(&FS<&FSMock>, &str), count: impl Fn(&Calls) -> usize, other_path: &str) {
    let underlying = create_mock_fs();
    let cached = from(&underlying);
    let n = || count(&underlying.calls.lock().unwrap());

    call(&cached, "/some/path");
    assert_eq!(1, n());

    call(&cached, "/some/path");
    assert_eq!(1, n());

    cached.clear_cache();
    call(&cached, "/some/path");
    assert_eq!(2, n());

    call(&cached, other_path);
    assert_eq!(3, n());

    cached.disable_and_clear_cache();
    call(&cached, "/some/path");
    assert_eq!(4, n());

    call(&cached, "/some/path");
    assert_eq!(5, n());

    cached.enable();
    call(&cached, "/some/path");
    assert_eq!(6, n());

    call(&cached, "/some/path");
    assert_eq!(6, n());
}

// Same, for methods that must never be cached.
fn check_uncached(call: impl Fn(&FS<&FSMock>), count: impl Fn(&Calls) -> usize) {
    let underlying = create_mock_fs();
    let cached = from(&underlying);
    let n = || count(&underlying.calls.lock().unwrap());

    call(&cached);
    assert_eq!(1, n());

    call(&cached);
    assert_eq!(2, n());

    cached.clear_cache();
    call(&cached);
    assert_eq!(3, n());

    cached.disable_and_clear_cache();
    call(&cached);
    assert_eq!(4, n());

    call(&cached);
    assert_eq!(5, n());

    cached.enable();
    call(&cached);
    assert_eq!(6, n());

    call(&cached);
    assert_eq!(7, n());
}

#[test]
fn test_directory_exists() {
    check_cached(
        |c, p| {
            c.directory_exists(p);
        },
        |c| c.directory_exists.len(),
        "/other/path",
    );
}

#[test]
fn test_file_exists() {
    check_cached(
        |c, p| {
            c.file_exists(&format!("{}/file.txt", p));
        },
        |c| c.file_exists.len(),
        "/other/path",
    );
}

#[test]
fn test_get_accessible_entries() {
    check_cached(
        |c, p| {
            c.get_accessible_entries(p);
        },
        |c| c.get_accessible_entries.len(),
        "/other/path",
    );
}

#[test]
fn test_realpath() {
    check_cached(
        |c, p| {
            c.realpath(p);
        },
        |c| c.realpath.len(),
        "/other/path",
    );
}

#[test]
fn test_stat() {
    check_cached(
        |c, p| {
            c.stat(p);
        },
        |c| c.stat.len(),
        "/other/path",
    );
}

#[test]
fn test_read_file() {
    check_uncached(
        |c| {
            c.read_file("/some/path/file.txt");
        },
        |c| c.read_file.len(),
    );
}

#[test]
fn test_use_case_sensitive_file_names() {
    check_uncached(
        |c| {
            c.use_case_sensitive_file_names();
        },
        |c| c.use_case_sensitive_file_names,
    );
}

#[test]
fn test_remove() {
    check_uncached(
        |c| {
            let _ = c.remove("/some/path/file.txt");
        },
        |c| c.remove.len(),
    );
}

#[test]
fn test_write_file() {
    let underlying = create_mock_fs();
    let cached = from(&underlying);
    let n = || underlying.calls.lock().unwrap().write_file.len();

    let _ = cached.write_file("/some/path/file.txt", "new content");
    assert_eq!(1, n());

    let _ = cached.write_file("/some/path/file.txt", "another content");
    assert_eq!(2, n());

    cached.clear_cache();
    let _ = cached.write_file("/some/path/file.txt", "third content");
    assert_eq!(3, n());

    let call = underlying.calls.lock().unwrap().write_file[2].clone();
    assert_eq!("/some/path/file.txt", call.0);
    assert_eq!("third content", call.1);

    cached.disable_and_clear_cache();
    let _ = cached.write_file("/some/path/file.txt", "fourth content");
    assert_eq!(4, n());

    let _ = cached.write_file("/some/path/file.txt", "fifth content");
    assert_eq!(5, n());

    cached.enable();
    let _ = cached.write_file("/some/path/file.txt", "sixth content");
    assert_eq!(6, n());

    let _ = cached.write_file("/some/path/file.txt", "seventh content");
    assert_eq!(7, n());
}
