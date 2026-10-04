use std::fs;
use std::io;
use std::sync::LazyLock;
use std::time::SystemTime;

use tsrs_core::tspath;

use crate::internal::{self, Common, IoDirEntry, IoFS};
use crate::{Entries, FileInfo, FileMode, FsError, FS};

// tsrs-only: file system calls made through this FS, printed as `--extendedDiagnostics` rows.
static STAT_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static READ_FILE_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static READ_DIR_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LSTAT_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn count(c: &std::sync::atomic::AtomicU64) {
    c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Adds the calls counted since the last call to the phase rows.
pub fn record_call_counts() {
    use std::sync::atomic::Ordering::Relaxed;
    for (name, c) in [
        ("FS: stat", &STAT_CALLS),
        ("FS: read file", &READ_FILE_CALLS),
        ("FS: read dir", &READ_DIR_CALLS),
        ("FS: realpath lstat", &LSTAT_CALLS),
    ] {
        let n = c.swap(0, Relaxed);
        if n > 0 {
            tsrs_core::phases::count(name, n);
        }
    }
}

// FS creates a new FS from the OS file system.
pub fn fs() -> &'static dyn FS {
    &*OS_VFS
}

static OS_VFS: LazyLock<OsFS> = LazyLock::new(|| OsFS {
    common: Common {
        root_for: Box::new(|root: &str| Some(Box::new(DirFS { dir: root.to_string() }) as Box<dyn IoFS>)),
        is_reparse_point: Some(is_reparse_point),
    },
});

pub struct OsFS {
    common: Common,
}

// Go computes this right at startup to minimize the chance that executable gets moved or deleted.
static IS_FILE_SYSTEM_CASE_SENSITIVE: LazyLock<bool> = LazyLock::new(|| {
    // win32/win64 are case insensitive platforms
    if cfg!(windows) {
        return false;
    }

    if cfg!(target_arch = "wasm32") {
        // !!! Who knows; this depends on the host implementation.
        return true;
    }

    // As a proxy for case-insensitivity, we check if the current executable exists under a different case.
    // This is not entirely correct, since different OSs can have differing case sensitivity in different paths,
    // but this is largely good enough for our purposes (and what sys.ts used to do with __filename).
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => panic!("vfs: failed to get executable path: {}", err),
    };
    let exe = exe.to_string_lossy().into_owned();

    // If the current executable exists under a different case, we must be case-insensitive.
    let swapped = swap_case(&exe);
    if let Err(err) = fs::metadata(&swapped) {
        if err.kind() == io::ErrorKind::NotFound {
            return true;
        }
        panic!("vfs: failed to stat {:?}: {}", swapped, err);
    }
    false
});

// Convert all lowercase chars to uppercase, and vice-versa
fn swap_case(str: &str) -> String {
    str.chars()
        .map(|r| {
            let upper = simple_to_upper(r);
            if upper == r {
                simple_to_lower(r)
            } else {
                upper
            }
        })
        .collect()
}

// unicode.ToUpper / unicode.ToLower map a rune to a single rune.
fn simple_to_upper(r: char) -> char {
    let mut it = r.to_uppercase();
    match (it.next(), it.next()) {
        (Some(c), None) => c,
        _ => r,
    }
}

fn simple_to_lower(r: char) -> char {
    let mut it = r.to_lowercase();
    match (it.next(), it.next()) {
        (Some(c), None) => c,
        _ => r,
    }
}

impl FS for OsFS {
    fn use_case_sensitive_file_names(&self) -> bool {
        *IS_FILE_SYSTEM_CASE_SENSITIVE
    }

    fn read_file(&self, path: &str) -> Option<String> {
        self.common.read_file(path)
    }

    fn directory_exists(&self, path: &str) -> bool {
        self.common.directory_exists(path)
    }

    fn file_exists(&self, path: &str) -> bool {
        self.common.file_exists(path)
    }

    fn get_accessible_entries(&self, path: &str) -> Entries {
        self.common.get_accessible_entries(path)
    }

    fn stat(&self, path: &str) -> Option<FileInfo> {
        self.common.stat(path)
    }

    fn realpath(&self, path: &str) -> String {
        os_fs_realpath(path)
    }

    fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        self.write_file_ensuring_dir(path, content, false)
    }

    fn append_file(&self, path: &str, content: &str) -> Result<(), String> {
        self.write_file_ensuring_dir(path, content, true)
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        // todo: #701 add retry mechanism?
        remove_all(path).map_err(|e| e.to_string())
    }

    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        let file = fs::File::options().write(true).open(path).or_else(|_| fs::File::open(path)).map_err(|e| e.to_string())?;
        file.set_times(fs::FileTimes::new().set_accessed(a_time).set_modified(m_time)).map_err(|e| e.to_string())
    }
}

fn os_fs_realpath(path: &str) -> String {
    let _ = internal::root_length(path); // Assert path is rooted

    let orig = path;
    let path = from_slash(path);
    let path = match native_realpath(&path) {
        Ok(p) => p,
        Err(_) => return orig.to_string(),
    };
    let path = match abs(&path) {
        Ok(p) => p,
        Err(_) => return orig.to_string(),
    };
    tspath::normalize_slashes(&path)
}

fn from_slash(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\")
    } else {
        path.to_string()
    }
}

// nativepath.Realpath: on Linux the kernel-resolved path (Go reads /proc/self/fd
// after an O_PATH open, which is what realpath(3) computes); elsewhere
// filepath.EvalSymlinks, which resolves symlinks without correcting case.
#[cfg(target_os = "linux")]
fn native_realpath(path: &str) -> io::Result<String> {
    Ok(fs::canonicalize(path)?.to_string_lossy().into_owned())
}

#[cfg(windows)]
fn native_realpath(path: &str) -> io::Result<String> {
    let p = fs::canonicalize(path)?.to_string_lossy().into_owned();
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        return Ok(format!(r"\\{}", rest));
    }
    if let Some(rest) = p.strip_prefix(r"\\?\") {
        return Ok(rest.to_string());
    }
    Ok(p)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn native_realpath(path: &str) -> io::Result<String> {
    walk_symlinks(path)
}

// filepath.walkSymlinks (the Unix variant, used by filepath.EvalSymlinks).
#[cfg(not(any(target_os = "linux", windows)))]
fn walk_symlinks(path: &str) -> io::Result<String> {
    let is_sep = |b: u8| b == b'/';
    let mut path = path.to_string();
    let mut vol_len = 0usize;

    if vol_len < path.len() && is_sep(path.as_bytes()[vol_len]) {
        vol_len += 1;
    }
    let mut vol = path[..vol_len].to_string();
    let mut dest = vol.clone();
    let mut links_walked = 0;
    let mut start = vol_len;
    while start < path.len() {
        let b = path.as_bytes();
        while start < b.len() && is_sep(b[start]) {
            start += 1;
        }
        let mut end = start;
        while end < b.len() && !is_sep(b[end]) {
            end += 1;
        }

        // The next path component is in path[start:end].
        if end == start {
            // No more path components.
            break;
        } else if &path[start..end] == "." {
            // Ignore path component ".".
            start = end;
            continue;
        } else if &path[start..end] == ".." {
            // Back up to previous component if possible.
            // Note that volLen includes any leading slash.

            // Set r to the index of the last slash in dest,
            // after the volume.
            let db = dest.as_bytes();
            let mut r = db.len() as isize - 1;
            while r >= vol_len as isize {
                if is_sep(db[r as usize]) {
                    break;
                }
                r -= 1;
            }
            if r < vol_len as isize || &dest[(r + 1) as usize..] == ".." {
                // Either path has no slashes
                // (it's empty or just "C:")
                // or it ends in a ".." we had to keep.
                // Either way, keep this "..".
                if dest.len() > vol_len {
                    dest.push('/');
                }
                dest.push_str("..");
            } else {
                // Discard everything since the last slash.
                dest.truncate(r as usize);
            }
            start = end;
            continue;
        }

        // Ordinary path component. Add it to result.

        if !dest.is_empty() && !is_sep(dest.as_bytes()[dest.len() - 1]) {
            dest.push('/');
        }

        dest.push_str(&path[start..end]);

        // Resolve symlink.

        count(&LSTAT_CALLS);
        let fi = fs::symlink_metadata(&dest)?;

        if !fi.file_type().is_symlink() {
            if !fi.is_dir() && end < path.len() {
                return Err(io::Error::from_raw_os_error(20 /* ENOTDIR */));
            }
            start = end;
            continue;
        }

        // Found symlink.

        links_walked += 1;
        if links_walked > 255 {
            return Err(io::Error::other("EvalSymlinks: too many links"));
        }

        let link = fs::read_link(&dest)?.to_string_lossy().into_owned();

        path = format!("{}{}", link, &path[end..]);

        if !link.is_empty() && is_sep(link.as_bytes()[0]) {
            // Symlink to absolute path.
            dest = link[..1].to_string();
            end = 1;
            vol = link[..1].to_string();
            vol_len = 1;
        } else {
            // Symlink to relative path; replace last
            // path component in dest.
            let db = dest.as_bytes();
            let mut r = db.len() as isize - 1;
            while r >= vol_len as isize {
                if is_sep(db[r as usize]) {
                    break;
                }
                r -= 1;
            }
            if r < vol_len as isize {
                dest = vol.clone();
            } else {
                dest.truncate(r as usize);
            }
            end = 0;
        }
        start = end;
    }
    Ok(clean(&dest))
}

// filepath.Abs
fn abs(path: &str) -> io::Result<String> {
    if std::path::Path::new(path).is_absolute() {
        return Ok(clean(path));
    }
    let wd = std::env::current_dir()?.to_string_lossy().into_owned();
    Ok(clean(&format!("{}/{}", wd, path)))
}

// path.Clean (Unix semantics; on Windows the realpath result is already clean).
pub(crate) fn clean(path: &str) -> String {
    if cfg!(windows) {
        return path.to_string();
    }
    if path.is_empty() {
        return ".".to_string();
    }

    let b = path.as_bytes();
    let rooted = b[0] == b'/';
    let n = b.len();

    let mut out: Vec<u8> = Vec::with_capacity(n);
    let mut r = 0;
    let mut dotdot = 0;
    if rooted {
        out.push(b'/');
        r = 1;
        dotdot = 1;
    }

    while r < n {
        if b[r] == b'/' {
            // empty path element
            r += 1;
        } else if b[r] == b'.' && (r + 1 == n || b[r + 1] == b'/') {
            // . element
            r += 1;
        } else if b[r] == b'.' && b[r + 1] == b'.' && (r + 2 == n || b[r + 2] == b'/') {
            // .. element: remove to last /
            r += 2;
            if out.len() > dotdot {
                // can backtrack
                let mut w = out.len() - 1;
                while w > dotdot && out[w] != b'/' {
                    w -= 1;
                }
                out.truncate(w);
            } else if !rooted {
                // cannot backtrack, but not rooted, so append .. element.
                if !out.is_empty() {
                    out.push(b'/');
                }
                out.extend_from_slice(b"..");
                dotdot = out.len();
            }
        } else {
            // real path element.
            // add slash if needed
            if (rooted && out.len() != 1) || (!rooted && !out.is_empty()) {
                out.push(b'/');
            }
            // copy element
            while r < n && b[r] != b'/' {
                out.push(b[r]);
                r += 1;
            }
        }
    }

    // Turn empty string into "."
    if out.is_empty() {
        return ".".to_string();
    }

    String::from_utf8(out).unwrap()
}

fn is_reparse_point(path: &str) -> bool {
    match fs::symlink_metadata(from_slash(path)) {
        Ok(info) => info.file_type().is_symlink(),
        Err(_) => false,
    }
}

fn remove_all(path: &str) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(md) if md.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    }
}

impl OsFS {
    fn write_file_with_flag(&self, path: &str, content: &str, append: bool) -> io::Result<()> {
        use std::io::Write;
        let mut options = fs::File::options();
        options.write(true).create(true);
        if append {
            options.append(true);
        } else {
            options.truncate(true);
        }
        let mut file = options.open(path)?;
        file.write_all(content.as_bytes())
    }

    fn ensure_directory_exists(&self, directory_path: &str) -> io::Result<()> {
        fs::create_dir_all(directory_path)
    }

    fn write_file_ensuring_dir(&self, path: &str, content: &str, append: bool) -> Result<(), String> {
        let _ = internal::root_length(path); // Assert path is rooted
        if self.write_file_with_flag(path, content, append).is_ok() {
            return Ok(());
        }
        self.ensure_directory_exists(&tspath::get_directory_path(&tspath::normalize_path(path))).map_err(|e| e.to_string())?;
        self.write_file_with_flag(path, content, append).map_err(|e| e.to_string())
    }
}

// DirFS is os.DirFS: an io/fs view of the OS file system rooted at dir.
struct DirFS {
    dir: String,
}

impl DirFS {
    fn join(&self, name: &str) -> Result<String, FsError> {
        if self.dir.is_empty() {
            return Err(FsError::Other("os: DirFS with empty root".to_string()));
        }
        if !internal::valid_path(name) || name.contains('\0') {
            return Err(FsError::Invalid);
        }
        let name = from_slash(name);
        let last = self.dir.as_bytes()[self.dir.len() - 1];
        if last == b'/' || (cfg!(windows) && last == b'\\') {
            return Ok(format!("{}{}", self.dir, name));
        }
        Ok(format!("{}{}{}", self.dir, std::path::MAIN_SEPARATOR, name))
    }
}

fn io_error(e: io::Error) -> FsError {
    match e.kind() {
        io::ErrorKind::NotFound => FsError::NotExist,
        io::ErrorKind::PermissionDenied => FsError::Permission,
        io::ErrorKind::AlreadyExists => FsError::Exist,
        _ => FsError::Other(e.to_string()),
    }
}

fn file_type_mode(ft: fs::FileType) -> FileMode {
    if ft.is_dir() {
        return FileMode::Dir;
    }
    if ft.is_symlink() {
        return FileMode::Symlink;
    }
    if ft.is_file() {
        return FileMode::None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if ft.is_fifo() {
            return FileMode::NamedPipe;
        }
        if ft.is_socket() {
            return FileMode::Socket;
        }
        if ft.is_char_device() {
            return FileMode::Device | FileMode::CharDevice;
        }
        if ft.is_block_device() {
            return FileMode::Device;
        }
    }
    FileMode::Irregular
}

fn metadata_mode(md: &fs::Metadata) -> FileMode {
    let mut mode = file_type_mode(md.file_type());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        mode |= FileMode::from_bits_retain(md.permissions().mode() & 0o777);
    }
    #[cfg(not(unix))]
    {
        if md.permissions().readonly() {
            mode |= FileMode::from_bits_retain(0o444);
        } else {
            mode |= FileMode::from_bits_retain(0o666);
        }
        if md.is_dir() {
            mode |= FileMode::from_bits_retain(0o111);
        }
    }
    mode
}

// filepathlite.Base
fn base(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let is_sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let trimmed = path.trim_end_matches(is_sep);
    if trimmed.is_empty() {
        return std::path::MAIN_SEPARATOR.to_string();
    }
    match trimmed.rfind(is_sep) {
        Some(i) => trimmed[i + 1..].to_string(),
        None => trimmed.to_string(),
    }
}

impl IoFS for DirFS {
    fn stat(&self, name: &str) -> Result<FileInfo, FsError> {
        let full = self.join(name)?;
        count(&STAT_CALLS);
        let md = fs::metadata(&full).map_err(io_error)?;
        Ok(FileInfo {
            name: base(&full),
            size: md.len() as i64,
            mode: metadata_mode(&md),
            mod_time: md.modified().ok(),
        })
    }

    fn read_dir(&self, name: &str) -> Result<Vec<IoDirEntry>, FsError> {
        let full = self.join(name)?;
        count(&READ_DIR_CALLS);
        let mut entries = Vec::new();
        for entry in fs::read_dir(&full).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let type_ = match entry.file_type() {
                Ok(ft) => file_type_mode(ft),
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_error(e)),
            };
            entries.push(IoDirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                type_,
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn read_file(&self, name: &str) -> Result<Vec<u8>, FsError> {
        let full = self.join(name)?;
        count(&READ_FILE_CALLS);
        fs::read(&full).map_err(io_error)
    }
}

#[cfg(test)]
#[path = "os_test.rs"]
mod os_test;

#[cfg(test)]
#[path = "realpath_test.rs"]
mod realpath_test;
