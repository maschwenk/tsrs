use std::fs;
use std::io;
use std::sync::LazyLock;
use std::time::SystemTime;

use tsrs_core::{stringutil, tspath};

use crate::fsstats;
use crate::internal::{self, Common, IoDirEntry, IoFS};
use crate::{Entries, FS, FileInfo, FileMode, FsError};

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "resolvercache.rs"]
mod resolvercache;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use resolvercache::for_compilation;

// tsrs-only: file system calls made through this FS, printed as `--extendedDiagnostics` rows.
static STAT_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static READ_FILE_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static READ_DIR_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LSTAT_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static READLINK_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static REALPATH_OPEN_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static STATFS_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn count(c: &std::sync::atomic::AtomicU64) {
    c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Counts a call that another file system over the OS made (the WebAssembly build's host file system), in the same
/// rows.
pub fn count_stat_call() {
    count(&STAT_CALLS);
}

pub fn count_read_file_call() {
    count(&READ_FILE_CALLS);
}

pub fn count_read_dir_call() {
    count(&READ_DIR_CALLS);
}

/// Adds the calls counted since the last call to the phase rows.
pub fn record_call_counts() {
    use std::sync::atomic::Ordering::Relaxed;
    for (name, c) in [
        ("FS: stat", &STAT_CALLS),
        ("FS: read file", &READ_FILE_CALLS),
        ("FS: read dir", &READ_DIR_CALLS),
        ("FS: lstat", &LSTAT_CALLS),
        ("FS: readlink", &READLINK_CALLS),
        ("FS: realpath open", &REALPATH_OPEN_CALLS),
        ("FS: statfs", &STATFS_CALLS),
    ] {
        let n = c.swap(0, Relaxed);
        if n > 0 {
            tsrs_core::phases::count(name, n);
        }
    }
    fsstats::write_report();
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
    let exe = match fsstats::physical("api.current_exe", "", std::env::current_exe, io_result) {
        Ok(exe) => exe,
        Err(err) => panic!("vfs: failed to get executable path: {}", err),
    };
    let exe = exe.to_string_lossy().into_owned();

    // If the current executable exists under a different case, we must be case-insensitive.
    let swapped = swap_case(&exe);
    if let Err(err) = fsstats::physical("api.metadata.case_probe", &swapped, || fs::metadata(&swapped), metadata_result) {
        if err.kind() == io::ErrorKind::NotFound {
            return true;
        }
        panic!("vfs: failed to stat {:?}: {}", swapped, err);
    }
    false
});

// Convert all lowercase chars to uppercase, and vice-versa
fn swap_case(str: &str) -> String {
    let mut out = String::with_capacity(str.len());
    for r in str.chars() {
        let r = r as stringutil::Rune;
        let upper = stringutil::unicode_to_upper(r);
        if upper == r {
            stringutil::push_rune(&mut out, stringutil::unicode_to_lower(r));
        } else {
            stringutil::push_rune(&mut out, upper);
        }
    }
    out
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
        let file = fsstats::physical("api.open.write", path, || fs::File::options().write(true).open(path), io_result)
            .or_else(|_| fsstats::physical("api.open.read", path, || fs::File::open(path), io_result))
            .map_err(|e| e.to_string())?;
        fsstats::physical(
            "api.File.set_times",
            path,
            || file.set_times(fs::FileTimes::new().set_accessed(a_time).set_modified(m_time)),
            io_result,
        )
        .map_err(|e| e.to_string())
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
    if cfg!(windows) { path.replace('/', "\\") } else { path.to_string() }
}

// nativepath.Realpath: Linux lets the kernel resolve the whole path with O_PATH,
// then reads it through /proc/self/fd. Elsewhere Go uses filepath.EvalSymlinks,
// which resolves symlinks without correcting case.
#[cfg(target_os = "linux")]
fn native_realpath(path: &str) -> io::Result<String> {
    static HAS_PROC_SELF_FD: LazyLock<bool> = LazyLock::new(|| {
        fsstats::physical("api.metadata.proc_probe", "/proc/self/fd/", || fs::metadata("/proc/self/fd/"), metadata_result).is_ok()
    });
    linux_realpath(path, *HAS_PROC_SELF_FD)
}

#[cfg(target_os = "linux")]
fn linux_realpath(path: &str, has_proc_self_fd: bool) -> io::Result<String> {
    use rustix::fs::{CWD, Mode, OFlags};
    use std::os::fd::AsRawFd;

    if !has_proc_self_fd {
        return walk_symlinks(path);
    }

    // OwnedFd closes on every return, including readlink errors. As in Go, only
    // the absence of procfs selects the fallback; open/readlink errors propagate.
    let fd = ignoring_eintr(|| {
        count(&REALPATH_OPEN_CALLS);
        fsstats::physical(
            "rustix.open.O_PATH",
            path,
            || rustix::fs::open(path, OFlags::PATH | OFlags::CLOEXEC, Mode::empty()),
            rustix_result,
        )
    })?;
    let proc_path = format!("/proc/self/fd/{}", fd.as_raw_fd());
    let mut buf = vec![0; 256];
    loop {
        let n = ignoring_eintr(|| {
            count(&READLINK_CALLS);
            fsstats::physical(
                "rustix.readlinkat_raw",
                &proc_path,
                || rustix::fs::readlinkat_raw(CWD, proc_path.as_str(), buf.as_mut_slice()),
                rustix_result,
            )
        })?;
        if n < buf.len() {
            buf.truncate(n);
            return Ok(String::from_utf8_lossy(&buf).into_owned());
        }
        buf.resize(buf.len() * 2, 0);
    }
}

#[cfg(target_os = "linux")]
fn ignoring_eintr<T>(mut operation: impl FnMut() -> rustix::io::Result<T>) -> io::Result<T> {
    loop {
        match operation() {
            Err(rustix::io::Errno::INTR) => {}
            result => return result.map_err(Into::into),
        }
    }
}

#[cfg(windows)]
fn native_realpath(path: &str) -> io::Result<String> {
    let p = fsstats::physical("api.canonicalize", path, || fs::canonicalize(path), io_result)?.to_string_lossy().into_owned();
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
#[cfg(not(windows))]
fn walk_symlinks(path: &str) -> io::Result<String> {
    walk_symlinks_with(path, symlink_file_type, read_link)
}

#[cfg(not(windows))]
pub(crate) fn symlink_file_type(path: &str) -> io::Result<FileMode> {
    count(&LSTAT_CALLS);
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{AtFlags, CWD, FileType, StatxFlags};

        match fsstats::physical(
            "rustix.statx.link",
            path,
            || rustix::fs::statx(CWD, path, AtFlags::SYMLINK_NOFOLLOW, StatxFlags::TYPE),
            rustix_result,
        ) {
            Ok(statx) => Ok(match FileType::from_raw_mode(statx.stx_mode.into()) {
                FileType::RegularFile => FileMode::None,
                FileType::Directory => FileMode::Dir,
                FileType::Symlink => FileMode::Symlink,
                FileType::Fifo => FileMode::NamedPipe,
                FileType::Socket => FileMode::Socket,
                FileType::CharacterDevice => FileMode::Device | FileMode::CharDevice,
                FileType::BlockDevice => FileMode::Device,
                FileType::Unknown => FileMode::Irregular,
            }),
            Err(rustix::io::Errno::NOSYS) => {
                fsstats::physical("api.symlink_metadata", path, || fs::symlink_metadata(path), metadata_result)
                    .map(|metadata| file_type_mode(metadata.file_type()))
            }
            Err(err) => Err(err.into()),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        fsstats::physical("api.symlink_metadata", path, || fs::symlink_metadata(path), metadata_result)
            .map(|metadata| file_type_mode(metadata.file_type()))
    }
}

#[cfg(not(windows))]
pub(crate) fn read_link(path: &str) -> io::Result<String> {
    count(&READLINK_CALLS);
    fsstats::physical("api.read_link", path, || fs::read_link(path), io_result).map(|target| target.to_string_lossy().into_owned())
}

// Unlike accessible entries, this keeps dangling links and special nodes. Only ordinary
// filesystems are eligible: procfs, for example, has lookup-visible TIDs absent from readdir.
// Any filesystem-query/listing error or non-ASCII name disables negative inference.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn directory_names(path: &str) -> Option<Vec<Box<str>>> {
    count(&STATFS_CALLS);
    let filesystem = fsstats::physical("rustix.statfs", path, || rustix::fs::statfs(path), rustix_result).ok()?;
    if !has_enumerated_names(&filesystem) {
        return None;
    }
    count(&READ_DIR_CALLS);
    let mut directory = fsstats::physical("api.read_dir", path, || fs::read_dir(path), io_result).ok()?;
    let mut names = Vec::new();
    loop {
        let entry = fsstats::physical("api.ReadDir.next", path, || directory.next(), |result| match result {
            Some(result) => io_result(result),
            None => "end",
        });
        let Some(entry) = entry else {
            return Some(names);
        };
        let name = entry.ok()?.file_name().into_string().ok()?;
        if !name.is_ascii() {
            return None;
        }
        names.push(name.into_boxed_str());
    }
}

#[cfg(target_os = "linux")]
fn has_enumerated_names(filesystem: &rustix::fs::StatFs) -> bool {
    // Linux include/uapi/linux/magic.h. Unknown types, including overlay/FUSE with arbitrary
    // underlying lookup semantics, must retain individual metadata probes.
    const EXT_SUPER_MAGIC: u32 = 0xef53;
    const XFS_SUPER_MAGIC: u32 = 0x5846_5342;
    const BTRFS_SUPER_MAGIC: u32 = 0x9123_683e;
    const TMPFS_MAGIC: u32 = 0x0102_1994;
    matches!(filesystem.f_type as u32, EXT_SUPER_MAGIC | XFS_SUPER_MAGIC | BTRFS_SUPER_MAGIC | TMPFS_MAGIC)
}

#[cfg(target_os = "macos")]
fn has_enumerated_names(filesystem: &rustix::fs::StatFs) -> bool {
    // f_fstypename is a fixed NUL-terminated C array; compare only complete known names,
    // without allocating or requiring zero padding after the terminator.
    let name = &filesystem.f_fstypename;
    name.starts_with(&[b'a' as _, b'p' as _, b'f' as _, b's' as _, 0]) || name.starts_with(&[b'h' as _, b'f' as _, b's' as _, 0])
}

// The invocation-scoped resolver cache supplies the same lstat/readlink views;
// path traversal and error handling remain the Go walk for both callers.
#[cfg(not(windows))]
pub(crate) fn walk_symlinks_with(
    path: &str,
    mut metadata: impl FnMut(&str) -> io::Result<FileMode>,
    mut read_link: impl FnMut(&str) -> io::Result<String>,
) -> io::Result<String> {
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
        start += (start..b.len()).take_while(|&index| is_sep(b[index])).count();
        let mut end = start;
        end += (end..b.len()).take_while(|&index| !is_sep(b[index])).count();

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

        let mode = metadata(&dest)?;

        if !mode.intersects(FileMode::Symlink) {
            if !mode.is_dir() && end < path.len() {
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

        let link = read_link(&dest)?;

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
                dest.clone_from(&vol);
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
    let wd = fsstats::physical("api.current_dir", "", std::env::current_dir, io_result)?.to_string_lossy().into_owned();
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
                w -= (dotdot + 1..=w).rev().take_while(|&i| out[i] != b'/').count();
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
            let end = r + b[r..n].iter().take_while(|&&byte| byte != b'/').count();
            out.extend_from_slice(&b[r..end]);
            r = end;
        }
    }

    // Turn empty string into "."
    if out.is_empty() {
        return ".".to_string();
    }

    tsrs_core::utf8::into_string(out).unwrap()
}

fn is_reparse_point(path: &str) -> bool {
    let path = from_slash(path);
    match fsstats::physical("api.symlink_metadata.reparse", &path, || fs::symlink_metadata(&path), metadata_result) {
        Ok(info) => info.file_type().is_symlink(),
        Err(_) => false,
    }
}

fn remove_all(path: &str) -> io::Result<()> {
    match fsstats::physical("api.symlink_metadata", path, || fs::symlink_metadata(path), metadata_result) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(md) if md.is_dir() => fsstats::physical("api.remove_dir_all", path, || fs::remove_dir_all(path), io_result),
        Ok(_) => fsstats::physical("api.remove_file", path, || fs::remove_file(path), io_result),
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
        let mut file = fsstats::physical("api.OpenOptions.open", path, || options.open(path), io_result)?;
        fsstats::physical("api.Write.write_all", path, || file.write_all(content.as_bytes()), io_result)
    }

    fn ensure_directory_exists(&self, directory_path: &str) -> io::Result<()> {
        fsstats::physical("api.create_dir_all", directory_path, || fs::create_dir_all(directory_path), io_result)
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
        // One allocation of the final size (module resolution stats and reads thousands of paths).
        let mut full = String::with_capacity(self.dir.len() + 1 + name.len());
        full.push_str(&self.dir);
        let last = self.dir.as_bytes()[self.dir.len() - 1];
        if !(last == b'/' || (cfg!(windows) && last == b'\\')) {
            full.push(std::path::MAIN_SEPARATOR);
        }
        if cfg!(windows) {
            full.push_str(&from_slash(name));
        } else {
            full.push_str(name);
        }
        Ok(full)
    }
}

fn error_result(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound => "error.not_found",
        io::ErrorKind::PermissionDenied => "error.permission_denied",
        io::ErrorKind::NotADirectory => "error.not_directory",
        io::ErrorKind::IsADirectory => "error.is_directory",
        io::ErrorKind::AlreadyExists => "error.already_exists",
        io::ErrorKind::InvalidInput => "error.invalid_input",
        io::ErrorKind::Interrupted => "error.interrupted",
        io::ErrorKind::Unsupported => "error.unsupported",
        _ => "error.other",
    }
}

fn io_result<T>(result: &io::Result<T>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(error) => error_result(error),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rustix_result<T>(result: &rustix::io::Result<T>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(error) => error_result(&(*error).into()),
    }
}

fn file_type_result(result: &io::Result<fs::FileType>) -> &'static str {
    match result {
        Ok(file_type) if file_type.is_dir() => "directory",
        Ok(file_type) if file_type.is_file() => "file",
        Ok(file_type) if file_type.is_symlink() => "symlink",
        Ok(_) => "special",
        Err(error) => error_result(error),
    }
}

fn metadata_result(result: &io::Result<fs::Metadata>) -> &'static str {
    match result {
        Ok(metadata) => file_type_result(&Ok(metadata.file_type())),
        Err(error) => error_result(error),
    }
}

#[expect(clippy::needless_pass_by_value, reason = "passed as `map_err(io_error)`, which hands over the error by value")]
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
        let md = fsstats::physical("api.metadata", &full, || fs::metadata(&full), metadata_result).map_err(io_error)?;
        Ok(FileInfo { name: base(&full), size: md.len() as i64, mode: metadata_mode(&md), mod_time: md.modified().ok() })
    }

    fn file_type(&self, name: &str) -> Result<FileMode, FsError> {
        let full = self.join(name)?;
        count(&STAT_CALLS);
        metadata_file_type(&full)
    }

    fn read_dir(&self, name: &str) -> Result<Vec<IoDirEntry>, FsError> {
        let full = self.join(name)?;
        count(&READ_DIR_CALLS);
        let mut entries = Vec::new();
        let mut directory = fsstats::physical("api.read_dir", &full, || fs::read_dir(&full), io_result).map_err(io_error)?;
        loop {
            let next = fsstats::physical(
                "api.ReadDir.next",
                &full,
                || directory.next(),
                |result| match result {
                    Some(result) => io_result(result),
                    None => "end",
                },
            );
            let Some(entry) = next else {
                break;
            };
            let entry = entry.map_err(io_error)?;
            // file_type may use the directory entry's type without any syscall. Construct its
            // diagnostic path only in profiling builds; it is not needed by the normal reader.
            #[cfg(feature = "resolver-fs-stats")]
            let entry_path = entry.path().to_string_lossy().into_owned();
            #[cfg(not(feature = "resolver-fs-stats"))]
            let entry_path = "";
            let type_ = match fsstats::physical("api.DirEntry.file_type", &entry_path, || entry.file_type(), file_type_result) {
                Ok(ft) => file_type_mode(ft),
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_error(e)),
            };
            entries.push(IoDirEntry { name: entry.file_name().to_string_lossy().into_owned(), type_ });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn read_file(&self, name: &str) -> Result<Vec<u8>, FsError> {
        let full = self.join(name)?;
        count(&READ_FILE_CALLS);
        fsstats::physical("api.read", &full, || fs::read(&full), io_result).map_err(io_error)
    }
}

#[cfg(target_os = "linux")]
fn metadata_file_type(path: &str) -> Result<FileMode, FsError> {
    use rustix::fs::{AtFlags, CWD, FileType, StatxFlags};

    match fsstats::physical("rustix.statx.follow", path, || rustix::fs::statx(CWD, path, AtFlags::empty(), StatxFlags::TYPE), rustix_result)
    {
        Ok(statx) => {
            let file_type = FileType::from_raw_mode(statx.stx_mode.into());
            Ok(if file_type.is_dir() { FileMode::Dir } else { FileMode::None })
        }
        Err(rustix::io::Errno::NOSYS) => fsstats::physical("api.metadata", path, || fs::metadata(path), metadata_result)
            .map(|md| file_type_mode(md.file_type()))
            .map_err(io_error),
        Err(err) => Err(io_error(err.into())),
    }
}

#[cfg(not(target_os = "linux"))]
fn metadata_file_type(path: &str) -> Result<FileMode, FsError> {
    fsstats::physical("api.metadata", path, || fs::metadata(path), metadata_result)
        .map(|md| file_type_mode(md.file_type()))
        .map_err(io_error)
}

#[cfg(test)]
#[path = "os_test.rs"]
mod os_test;

#[cfg(test)]
#[path = "realpath_test.rs"]
mod realpath_test;
