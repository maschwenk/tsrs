// Port of tsc/internal/api/callbackfs.go: a filesystem that delegates selected operations to the client
// (`--callbacks=readFile,fileExists,...`). Like Go, an invalid callback response or a configured
// `:error` callback panics; the session turns the panic into an error response for the request.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustc_hash::FxHashSet;
use tsrs_core::json::{self, Value};
use tsrs_core::tspath;
use tsrs_vfs::{Entries, FileInfo, FileMode, FS};

use crate::handler::ClientConn;
use crate::wire::{s, Obj};

const CALLBACK_NAMES: &[&str] = &["readFile", "fileExists", "directoryExists", "getAccessibleEntries", "realpath", "stat", "writeFile", "removeFile"];

pub struct CallbackFs {
    base: Arc<dyn FS>,
    enabled: HashSet<String>,
    realpath_identity: bool,
    fake_stat: bool,
    write_file_noop: bool,
    remove_file_noop: bool,
    error_callbacks: HashSet<String>,
    case_sensitive: Option<bool>,
    conn: OnceLock<Arc<dyn ClientConn>>,
}

impl CallbackFs {
    /// Go `newCallbackFS`; unknown callback names are an error (Go panics at startup).
    pub fn new(base: Arc<dyn FS>, callbacks: &[String], case_sensitive: Option<bool>) -> Result<CallbackFs, String> {
        let mut enabled = HashSet::new();
        let mut error_callbacks = HashSet::new();
        for cb in callbacks {
            if let Some(name) = cb.strip_suffix(":error") {
                if !CALLBACK_NAMES.contains(&name) {
                    return Err(format!("unknown callback name: {name}"));
                }
                error_callbacks.insert(name.to_string());
                continue;
            }
            if matches!(cb.as_str(), "realpath:identity" | "stat:fakeStat" | "writeFile:noop" | "removeFile:noop") {
                continue;
            }
            if !CALLBACK_NAMES.contains(&cb.as_str()) {
                return Err(format!("unknown callback name: {cb}"));
            }
            enabled.insert(cb.clone());
        }
        let has = |n: &str| callbacks.iter().any(|c| c == n);
        Ok(CallbackFs {
            base,
            enabled,
            realpath_identity: has("realpath:identity"),
            fake_stat: has("stat:fakeStat"),
            write_file_noop: has("writeFile:noop"),
            remove_file_noop: has("removeFile:noop"),
            error_callbacks,
            case_sensitive,
            conn: OnceLock::new(),
        })
    }

    /// Go `SetConnection`. Only the first connection is kept.
    pub fn set_connection(&self, conn: Arc<dyn ClientConn>) {
        let _ = self.conn.set(conn);
    }

    fn is_enabled(&self, name: &str) -> bool {
        self.enabled.contains(name)
    }

    fn panic_if_error(&self, name: &str) {
        if self.error_callbacks.contains(name) {
            panic!("filesystem operation configured with serverFS.error: {name}");
        }
    }

    fn call(&self, name: &str, arg: &Value) -> Result<String, String> {
        let conn = self.conn.get().ok_or_else(|| format!("CallbackFS: {name} called before connection set"))?;
        conn.call(name, &json::marshal(arg).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }

    /// Go `decodeCallbackResponse`: returns (kind, value).
    fn decode(name: &str, result: &str) -> (String, Value) {
        let v = json::unmarshal(result).unwrap_or_else(|e| panic!("{e}"));
        let Value::Object(o) = &v else { panic!("json: cannot unmarshal into callbackResponse") };
        let kind = match o.get("kind") {
            Some(Value::String(k)) if !k.is_empty() => k.clone(),
            _ => panic!("filesystem callback response is missing a kind"),
        };
        if kind == "error" {
            panic!("filesystem callback returned serverFS.error: {name}");
        }
        (kind, o.get("value").cloned().unwrap_or(Value::Null))
    }

    fn call_path(&self, name: &str, path: &str) -> (String, Value) {
        let result = self.call(name, &s(path)).unwrap_or_else(|e| panic!("{e}"));
        Self::decode(name, &result)
    }

    fn invalid(name: &str, kind: &str) -> ! {
        panic!("invalid {name} callback response kind: {kind}")
    }

    fn fake_stat_for_path(&self, path: &str) -> Option<FileInfo> {
        if self.directory_exists(path) {
            return Some(FileInfo { name: tspath::get_base_file_name(path), size: 0, mode: FileMode::Dir | FileMode::from_bits_retain(0o555), mod_time: None });
        }
        if self.file_exists(path) {
            return Some(FileInfo { name: tspath::get_base_file_name(path), size: 0, mode: FileMode::from_bits_retain(0o444), mod_time: None });
        }
        None
    }
}

fn expect_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        _ => panic!("json: cannot unmarshal into bool"),
    }
}

fn expect_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        _ => panic!("json: cannot unmarshal into string"),
    }
}

fn string_list(v: Option<&Value>) -> Option<Vec<String>> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(a.iter().map(expect_string).collect()),
        _ => panic!("json: cannot unmarshal into []string"),
    }
}

/// Go `nodeFileModeToGoFileMode`.
fn node_file_mode(mode: u32) -> FileMode {
    let mut r = FileMode::from_bits_retain(mode & 0o777);
    if mode & 0o4000 != 0 {
        r |= FileMode::Setuid;
    }
    if mode & 0o2000 != 0 {
        r |= FileMode::Setgid;
    }
    if mode & 0o1000 != 0 {
        r |= FileMode::Sticky;
    }
    r |= match mode & 0o170000 {
        0o010000 => FileMode::NamedPipe,
        0o020000 => FileMode::Device | FileMode::CharDevice,
        0o040000 => FileMode::Dir,
        0o060000 => FileMode::Device,
        0o100000 => FileMode::None,
        0o120000 => FileMode::Symlink,
        0o140000 => FileMode::Socket,
        _ => FileMode::Irregular,
    };
    r
}

/// `time.Parse(time.RFC3339Nano, s)` for the UTC/offset timestamps Node's `Date.toISOString()` produces.
fn parse_rfc3339(text: &str) -> Option<SystemTime> {
    let b = text.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b't') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { text.get(r)?.parse().ok() };
    let (y, mo, d, h, mi, sec) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let mut i = 19;
    let mut nanos: i64 = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac = &text[start..i];
        if frac.is_empty() || frac.len() > 9 {
            return None;
        }
        nanos = frac.parse::<i64>().ok()? * 10i64.pow(9 - frac.len() as u32);
    }
    let offset = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        b'+' | b'-' if i + 6 == b.len() && b[i + 3] == b':' => {
            let sign = if b[i] == b'-' { -1 } else { 1 };
            sign * (num(i + 1..i + 3)? * 3600 + num(i + 4..i + 6)? * 60)
        }
        _ => return None,
    };
    // Days from civil (Howard Hinnant's algorithm).
    let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + h * 3600 + mi * 60 + sec - offset;
    if secs >= 0 {
        Some(UNIX_EPOCH + Duration::new(secs as u64, nanos as u32))
    } else {
        UNIX_EPOCH.checked_sub(Duration::new((-secs) as u64, 0))?.checked_add(Duration::from_nanos(nanos as u64))
    }
}

impl FS for CallbackFs {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.case_sensitive.unwrap_or_else(|| self.base.use_case_sensitive_file_names())
    }

    fn read_file(&self, path: &str) -> Option<String> {
        self.panic_if_error("readFile");
        if self.is_enabled("readFile") {
            let (kind, value) = self.call_path("readFile", path);
            return match kind.as_str() {
                "value" => Some(expect_string(&value)),
                "missing" => None,
                "useOS" => self.base.read_file(path),
                k => Self::invalid("readFile", k),
            };
        }
        self.base.read_file(path)
    }

    fn file_exists(&self, path: &str) -> bool {
        self.panic_if_error("fileExists");
        if self.is_enabled("fileExists") {
            let (kind, value) = self.call_path("fileExists", path);
            return match kind.as_str() {
                "value" => expect_bool(&value),
                "useOS" => self.base.file_exists(path),
                k => Self::invalid("fileExists", k),
            };
        }
        self.base.file_exists(path)
    }

    fn directory_exists(&self, path: &str) -> bool {
        self.panic_if_error("directoryExists");
        if self.is_enabled("directoryExists") {
            let (kind, value) = self.call_path("directoryExists", path);
            return match kind.as_str() {
                "value" => expect_bool(&value),
                "useOS" => self.base.directory_exists(path),
                k => Self::invalid("directoryExists", k),
            };
        }
        self.base.directory_exists(path)
    }

    fn get_accessible_entries(&self, path: &str) -> Entries {
        self.panic_if_error("getAccessibleEntries");
        if self.is_enabled("getAccessibleEntries") {
            let (kind, value) = self.call_path("getAccessibleEntries", path);
            return match kind.as_str() {
                "value" => {
                    let Value::Object(o) = &value else { panic!("json: cannot unmarshal into entries") };
                    Entries {
                        files: string_list(o.get("files")).unwrap_or_default(),
                        directories: string_list(o.get("directories")).unwrap_or_default(),
                        symlinks: string_list(o.get("symlinks")).map(|l| l.into_iter().collect::<FxHashSet<_>>()),
                    }
                }
                "useOS" => self.base.get_accessible_entries(path),
                k => Self::invalid("getAccessibleEntries", k),
            };
        }
        self.base.get_accessible_entries(path)
    }

    fn realpath(&self, path: &str) -> String {
        self.panic_if_error("realpath");
        if self.is_enabled("realpath") {
            let (kind, value) = self.call_path("realpath", path);
            return match kind.as_str() {
                "value" => expect_string(&value),
                "identity" => path.to_string(),
                "useOS" => self.base.realpath(path),
                k => Self::invalid("realpath", k),
            };
        }
        if self.realpath_identity {
            return path.to_string();
        }
        self.base.realpath(path)
    }

    fn stat(&self, path: &str) -> Option<FileInfo> {
        self.panic_if_error("stat");
        if self.is_enabled("stat") {
            let (kind, value) = self.call_path("stat", path);
            return match kind.as_str() {
                "value" => {
                    let Value::Object(o) = &value else { panic!("json: cannot unmarshal into stat") };
                    let mode = match o.get("mode") {
                        Some(Value::Number(n)) => *n as u32,
                        _ => 0,
                    };
                    let size = match o.get("size") {
                        Some(Value::Number(n)) => *n as i64,
                        _ => 0,
                    };
                    let mtime = match o.get("mtime") {
                        Some(Value::String(t)) => t.clone(),
                        _ => String::new(),
                    };
                    let mod_time = parse_rfc3339(&mtime).unwrap_or_else(|| panic!("parsing time {mtime:?} as RFC3339Nano: cannot parse"));
                    Some(FileInfo { name: tspath::get_base_file_name(path), size, mode: node_file_mode(mode), mod_time: Some(mod_time) })
                }
                "missing" => None,
                "fakeStat" => self.fake_stat_for_path(path),
                "useOS" => self.base.stat(path),
                k => Self::invalid("stat", k),
            };
        }
        if self.fake_stat {
            return self.fake_stat_for_path(path);
        }
        self.base.stat(path)
    }

    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.panic_if_error("writeFile");
        if self.is_enabled("writeFile") {
            let result = self.call("writeFile", &Obj::new().set("path", s(path)).set("data", s(data)).build())?;
            let (kind, _) = Self::decode("writeFile", &result);
            return match kind.as_str() {
                "value" | "noop" => Ok(()),
                "useOS" => self.base.write_file(path, data),
                k => Self::invalid("writeFile", k),
            };
        }
        if self.write_file_noop {
            return Ok(());
        }
        self.base.write_file(path, data)
    }

    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.base.append_file(path, data)
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        self.panic_if_error("removeFile");
        if self.is_enabled("removeFile") {
            let result = self.call("removeFile", &s(path))?;
            let (kind, _) = Self::decode("removeFile", &result);
            return match kind.as_str() {
                "value" | "noop" => Ok(()),
                "useOS" => self.base.remove(path),
                k => Self::invalid("removeFile", k),
            };
        }
        if self.remove_file_noop {
            return Ok(());
        }
        self.base.remove(path)
    }

    fn chtimes(&self, path: &str, a: SystemTime, m: SystemTime) -> Result<(), String> {
        self.base.chtimes(path, a, m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:01.5Z"), Some(UNIX_EPOCH + Duration::from_millis(1500)));
        assert_eq!(parse_rfc3339("2024-02-29T12:00:00Z"), Some(UNIX_EPOCH + Duration::from_secs(1709208000)));
        assert_eq!(parse_rfc3339("2024-02-29T13:00:00+01:00"), Some(UNIX_EPOCH + Duration::from_secs(1709208000)));
        assert_eq!(parse_rfc3339("garbage"), None);
    }

    #[test]
    fn node_modes() {
        assert!(node_file_mode(0o040755).is_dir());
        assert!(node_file_mode(0o100644).is_regular());
        assert!(node_file_mode(0o120777).contains(FileMode::Symlink));
    }
}
