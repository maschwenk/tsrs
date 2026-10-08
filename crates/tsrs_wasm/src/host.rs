// The host file system: every file operation is a call to the JS host (`tsrs_host.fs` / `tsrs_host.fs_take`), so the
// same module serves Node's real file system, an in-memory map and browsers. Reads go through tsrs_vfs's `Common`
// (as osvfs does), so BOM handling, UTF-16 decoding and symlink following are the native code.
//
// Protocol: `fs(op, ptr, len)` gets the op's argument bytes and returns the result length n >= 0 (the host stages n
// bytes, which `fs_take(ptr)` copies into a buffer of n bytes), -1 for "not found", or -2 - n with an n-byte error
// text staged. Ops and their argument / result bytes:
//   0 Read      path                          -> file bytes
//   1 Stat      path                          -> "<f|d|o> <size> <mtime ns>" (symlinks followed)
//   2 ReadDir   path                          -> "<f|d|l|o><name>\0" per entry (l = symlink, o = other)
//   3 Realpath  path                          -> the real path
//   4 Write     path \0 data                  -> "" (the host creates missing parent directories)
//   5 Append    path \0 data                  -> ""
//   6 Remove    path                          -> "" (recursive; a missing path is not an error)
//   7 Chtimes   path \0 atime ns \0 mtime ns   -> ""
// Write errors are the texts native tsrs prints (Rust's `io::Error` display, "<strerror> (os error N)").

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tsrs_core::tspath;
use tsrs_vfs::internal::{self, Common, IoDirEntry, IoFS};
use tsrs_vfs::{Entries, FileInfo, FileMode, FsError, FS};

pub const OP_READ: u32 = 0;
pub const OP_STAT: u32 = 1;
pub const OP_READ_DIR: u32 = 2;
pub const OP_REALPATH: u32 = 3;
pub const OP_WRITE: u32 = 4;
pub const OP_APPEND: u32 = 5;
pub const OP_REMOVE: u32 = 6;
pub const OP_CHTIMES: u32 = 7;

pub enum HostError {
    NotFound,
    Other(String),
}

#[cfg(target_family = "wasm")]
#[link(wasm_import_module = "tsrs_host")]
extern "C" {
    fn fs(op: u32, ptr: *const u8, len: usize) -> i32;
    fn fs_take(ptr: *mut u8);
}

/// One host call: the op's argument bytes in, its result bytes (or error) out.
#[cfg(target_family = "wasm")]
pub fn call(op: u32, arg: &[u8]) -> Result<Vec<u8>, HostError> {
    // SAFETY: the host reads `arg.len()` bytes at `arg.as_ptr()` during the call and keeps no reference to them.
    let r = unsafe { fs(op, arg.as_ptr(), arg.len()) };
    if r == -1 {
        return Err(HostError::NotFound);
    }
    let n = if r >= 0 { r as usize } else { (-2 - r) as usize };
    let mut out = vec![0u8; n];
    // SAFETY: the host staged exactly `n` bytes for this call; `fs_take` copies them into `out`, which has `n` bytes.
    unsafe { fs_take(out.as_mut_ptr()) };
    if r >= 0 {
        Ok(out)
    } else {
        Err(HostError::Other(String::from_utf8_lossy(&out).into_owned()))
    }
}

#[cfg(not(target_family = "wasm"))]
pub use test_host::call;

/// Native builds (the crate's tests): an in-memory host behind the same protocol. Paths are absolute keys; a
/// directory exists when a file key is below it.
#[cfg(not(target_family = "wasm"))]
pub mod test_host {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;

    pub static FILES: Mutex<BTreeMap<String, Vec<u8>>> = Mutex::new(BTreeMap::new());
    /// Paths whose writes fail with this text (the TS5033 test).
    pub static FAIL_WRITES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

    fn is_dir(files: &BTreeMap<String, Vec<u8>>, path: &str) -> bool {
        let prefix = if path.ends_with('/') { path.to_string() } else { format!("{path}/") };
        files.range(prefix.clone()..).next().is_some_and(|(k, _)| k.starts_with(&prefix))
    }

    fn split_nul(arg: &[u8]) -> (String, &[u8]) {
        let i = arg.iter().position(|&b| b == 0).unwrap_or(arg.len());
        (String::from_utf8_lossy(&arg[..i]).into_owned(), arg.get(i + 1..).unwrap_or(&[]))
    }

    pub fn call(op: u32, arg: &[u8]) -> Result<Vec<u8>, HostError> {
        let mut files = FILES.lock().unwrap();
        let (path, rest) = split_nul(arg);
        match op {
            OP_READ => files.get(&path).cloned().ok_or(HostError::NotFound),
            OP_STAT => match files.get(&path) {
                Some(data) => Ok(format!("f {} 0", data.len()).into_bytes()),
                None if is_dir(&files, &path) || path == "/" => Ok(b"d 0 0".to_vec()),
                None => Err(HostError::NotFound),
            },
            OP_READ_DIR => {
                if !is_dir(&files, &path) && path != "/" {
                    return Err(HostError::NotFound);
                }
                let prefix = if path.ends_with('/') { path } else { format!("{path}/") };
                let mut names: BTreeMap<String, u8> = BTreeMap::new();
                for key in files.keys().filter(|k| k.starts_with(&prefix)) {
                    let tail = &key[prefix.len()..];
                    match tail.split_once('/') {
                        Some((dir, _)) => names.insert(dir.to_string(), b'd'),
                        None => names.insert(tail.to_string(), b'f'),
                    };
                }
                let mut out = Vec::new();
                for (name, kind) in names {
                    out.push(kind);
                    out.extend_from_slice(name.as_bytes());
                    out.push(0);
                }
                Ok(out)
            }
            OP_REALPATH => Ok(path.into_bytes()),
            OP_WRITE | OP_APPEND => {
                if let Some((_, text)) = FAIL_WRITES.lock().unwrap().iter().find(|(p, _)| path.starts_with(p.as_str())) {
                    return Err(HostError::Other(text.clone()));
                }
                let entry = files.entry(path).or_default();
                if op == OP_WRITE {
                    entry.clear();
                }
                entry.extend_from_slice(rest);
                Ok(Vec::new())
            }
            OP_REMOVE => {
                let prefix = format!("{path}/");
                files.retain(|k, _| k != &path && !k.starts_with(&prefix));
                Ok(Vec::new())
            }
            OP_CHTIMES => Ok(Vec::new()),
            _ => Err(HostError::Other(format!("unknown op {op}"))),
        }
    }
}

fn fs_error(e: HostError) -> FsError {
    match e {
        HostError::NotFound => FsError::NotExist,
        HostError::Other(text) => FsError::Other(text),
    }
}

fn error_text(e: HostError) -> String {
    match e {
        HostError::NotFound => "No such file or directory (os error 2)".to_string(),
        HostError::Other(text) => text,
    }
}

fn path_and_data(path: &str, data: &[u8]) -> Vec<u8> {
    let mut arg = Vec::with_capacity(path.len() + 1 + data.len());
    arg.extend_from_slice(path.as_bytes());
    arg.push(0);
    arg.extend_from_slice(data);
    arg
}

fn nanos(t: SystemTime) -> u128 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

/// The io/fs view of the host file system below one root (osvfs's `DirFS`).
struct HostRoot {
    root: String,
}

impl HostRoot {
    fn join(&self, name: &str) -> Result<String, FsError> {
        if !internal::valid_path(name) || name.contains('\0') {
            return Err(FsError::Invalid);
        }
        if name == "." {
            return Ok(self.root.clone());
        }
        let mut full = String::with_capacity(self.root.len() + 1 + name.len());
        full.push_str(&self.root);
        if !self.root.ends_with('/') {
            full.push('/');
        }
        full.push_str(name);
        Ok(full)
    }
}

fn base(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed).to_string()
}

impl IoFS for HostRoot {
    fn stat(&self, name: &str) -> Result<FileInfo, FsError> {
        let full = self.join(name)?;
        tsrs_vfs::osvfs::count_stat_call();
        let out = call(OP_STAT, full.as_bytes()).map_err(fs_error)?;
        let text = String::from_utf8_lossy(&out);
        let mut fields = text.split(' ');
        let mode = match fields.next() {
            Some("d") => FileMode::Dir | FileMode::from_bits_retain(0o755),
            Some("f") => FileMode::from_bits_retain(0o644),
            _ => FileMode::Irregular,
        };
        let size = fields.next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
        let mtime = fields.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        Ok(FileInfo { name: base(&full), size, mode, mod_time: Some(UNIX_EPOCH + Duration::from_nanos(mtime)) })
    }

    fn read_dir(&self, name: &str) -> Result<Vec<IoDirEntry>, FsError> {
        let full = self.join(name)?;
        tsrs_vfs::osvfs::count_read_dir_call();
        let out = call(OP_READ_DIR, full.as_bytes()).map_err(fs_error)?;
        let mut entries: Vec<IoDirEntry> = out
            .split(|&b| b == 0)
            .filter(|e| !e.is_empty())
            .map(|e| IoDirEntry {
                name: String::from_utf8_lossy(&e[1..]).into_owned(),
                type_: match e[0] {
                    b'd' => FileMode::Dir,
                    b'f' => FileMode::None,
                    b'l' => FileMode::Symlink,
                    _ => FileMode::Irregular,
                },
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn read_file(&self, name: &str) -> Result<Vec<u8>, FsError> {
        let full = self.join(name)?;
        tsrs_vfs::osvfs::count_read_file_call();
        call(OP_READ, full.as_bytes()).map_err(fs_error)
    }
}

/// `tsrs_vfs::FS` over the host (osvfs's `OsFS`, with host calls in place of `std::fs`).
pub struct HostFs {
    common: Common,
    case_sensitive: bool,
}

impl HostFs {
    pub fn new(case_sensitive: bool) -> HostFs {
        HostFs {
            common: Common {
                root_for: Box::new(|root: &str| Some(Box::new(HostRoot { root: root.to_string() }) as Box<dyn IoFS>)),
                // Only consulted for `Irregular` entries; the host reports symlinks as `l`.
                is_reparse_point: None,
            },
            case_sensitive,
        }
    }

    fn write(&self, op: u32, path: &str, content: &str) -> Result<(), String> {
        let _ = internal::root_length(path); // Assert path is rooted
        call(op, &path_and_data(path, content.as_bytes())).map(|_| ()).map_err(error_text)
    }
}

impl FS for HostFs {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.case_sensitive
    }

    fn file_exists(&self, path: &str) -> bool {
        self.common.file_exists(path)
    }

    fn read_file(&self, path: &str) -> Option<String> {
        self.common.read_file(path)
    }

    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.write(OP_WRITE, path, data)
    }

    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.write(OP_APPEND, path, data)
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        call(OP_REMOVE, path.as_bytes()).map(|_| ()).map_err(error_text)
    }

    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        let arg = format!("{path}\0{}\0{}", nanos(a_time), nanos(m_time));
        call(OP_CHTIMES, arg.as_bytes()).map(|_| ()).map_err(error_text)
    }

    fn directory_exists(&self, path: &str) -> bool {
        self.common.directory_exists(path)
    }

    fn get_accessible_entries(&self, path: &str) -> Entries {
        self.common.get_accessible_entries(path)
    }

    fn stat(&self, path: &str) -> Option<FileInfo> {
        self.common.stat(path)
    }

    fn realpath(&self, path: &str) -> String {
        let _ = internal::root_length(path); // Assert path is rooted
        match call(OP_REALPATH, path.as_bytes()) {
            Ok(real) => tspath::normalize_slashes(&String::from_utf8_lossy(&real)),
            Err(_) => path.to_string(),
        }
    }
}
