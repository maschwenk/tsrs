use std::sync::Arc;

use rustc_hash::FxHashSet;
use tsrs_core::tspath;

use crate::{Entries, FileInfo, FileMode, FsError};

// IoFS is the subset of Go's io/fs.FS (Open, plus the fs.Stat / fs.ReadDir /
// fs.ReadFile helpers) that the vfs implementations are built on. Names are
// io/fs names: slash-separated, unrooted, validated with valid_path.
//
// The as_* methods stand in for Go's interface type assertions
// (fsys.(iovfs.RealpathFS), fsys.(iovfs.WritableFS)).
pub trait IoFS: Send + Sync {
    fn stat(&self, name: &str) -> Result<FileInfo, FsError>;

    // ReadDir returns the entries sorted by filename, like fs.ReadDir.
    fn read_dir(&self, name: &str) -> Result<Vec<IoDirEntry>, FsError>;

    fn read_file(&self, name: &str) -> Result<Vec<u8>, FsError>;

    fn as_realpath_fs(&self) -> Option<&dyn crate::iovfs::RealpathFS> {
        None
    }

    fn as_writable_fs(&self) -> Option<&dyn crate::iovfs::WritableFS> {
        None
    }
}

impl<T: IoFS + ?Sized> IoFS for Arc<T> {
    fn stat(&self, name: &str) -> Result<FileInfo, FsError> {
        (**self).stat(name)
    }

    fn read_dir(&self, name: &str) -> Result<Vec<IoDirEntry>, FsError> {
        (**self).read_dir(name)
    }

    fn read_file(&self, name: &str) -> Result<Vec<u8>, FsError> {
        (**self).read_file(name)
    }

    fn as_realpath_fs(&self) -> Option<&dyn crate::iovfs::RealpathFS> {
        (**self).as_realpath_fs()
    }

    fn as_writable_fs(&self) -> Option<&dyn crate::iovfs::WritableFS> {
        (**self).as_writable_fs()
    }
}

// IoDirEntry is the part of fs.DirEntry that Common uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoDirEntry {
    pub name: String,
    pub type_: FileMode,
}

// ValidPath is fs.ValidPath.
pub fn valid_path(name: &str) -> bool {
    if name == "." {
        // special case
        return true;
    }

    // No element is empty, "." or "..": the first and last are not empty (an empty name is one empty element), and
    // no element is "." or ".." or empty between two slashes (one scan instead of a search per element).
    !name.is_empty() && !name.starts_with('/') && !name.ends_with('/') && !tspath::has_relative_path_segment(name)
}

pub type RootForFn = Box<dyn Fn(&str) -> Option<Box<dyn IoFS>> + Send + Sync>;

pub struct Common {
    pub root_for: RootForFn,
    pub is_reparse_point: Option<fn(&str) -> bool>,
}

pub fn root_length(p: &str) -> usize {
    let l = tspath::get_encoded_root_length(p);
    if l == 0 {
        panic!("vfs: path {:?} is not absolute", p);
    } else if l < 0 {
        return (!l) as usize;
    }
    l as usize
}

pub fn split_path(p: &str) -> (String, String) {
    let p = tspath::normalize_path(p);
    let l = root_length(&p);
    let root_name = p[..l].to_string();
    let rest = tspath::remove_trailing_directory_separator(&p[l..]).to_string();
    (root_name, rest)
}

impl Common {
    pub fn root_and_path(&self, path: &str) -> (Option<Box<dyn IoFS>>, String, String) {
        let (root_name, mut rest) = split_path(path);
        if rest.is_empty() {
            rest = ".".to_string();
        }
        ((self.root_for)(&root_name), root_name, rest)
    }

    pub fn stat(&self, path: &str) -> Option<FileInfo> {
        let (fsys, _, rest) = self.root_and_path(path);
        let fsys = fsys?;
        fsys.stat(&rest).ok()
    }

    pub fn file_exists(&self, path: &str) -> bool {
        let stat = self.stat(path);
        matches!(stat, Some(stat) if !stat.is_dir())
    }

    pub fn directory_exists(&self, path: &str) -> bool {
        let stat = self.stat(path);
        matches!(stat, Some(stat) if stat.is_dir())
    }

    pub fn get_accessible_entries(&self, path: &str) -> Entries {
        let mut result = Entries::default();
        let mut symlinks = FxHashSet::default();

        let add_to_result = |result: &mut Entries, symlinks: &mut FxHashSet<String>, name: &str, mode: FileMode, is_link: bool| -> bool {
            if mode.is_dir() {
                result.directories.push(name.to_string());
            } else if mode.is_regular() {
                result.files.push(name.to_string());
            } else {
                return false;
            }

            if is_link {
                symlinks.insert(name.to_string());
            }
            true
        };

        for entry in self.get_entries(path) {
            let entry_type = entry.type_;

            if add_to_result(&mut result, &mut symlinks, &entry.name, entry_type, false) {
                continue;
            }

            if entry_type.intersects(FileMode::Symlink) {
                // Easy case; UNIX-like system will clearly mark symlinks.
                if let Some(stat) = self.stat(&format!("{}/{}", path, entry.name)) {
                    add_to_result(&mut result, &mut symlinks, &entry.name, stat.mode, true);
                }
                continue;
            }

            if entry_type.intersects(FileMode::Irregular) {
                if let Some(is_reparse_point) = self.is_reparse_point {
                    // Could be a Windows junction or other reparse point.
                    // Check using the OS-specific helper.
                    let full_path = format!("{}/{}", path, entry.name);
                    if is_reparse_point(&full_path) {
                        if let Some(stat) = self.stat(&full_path) {
                            add_to_result(&mut result, &mut symlinks, &entry.name, stat.mode, true);
                        }
                    }
                    continue;
                }
            }
        }

        result.symlinks = Some(symlinks);
        result
    }

    fn get_entries(&self, path: &str) -> Vec<IoDirEntry> {
        let (fsys, _, rest) = self.root_and_path(path);
        let Some(fsys) = fsys else {
            return Vec::new();
        };

        fsys.read_dir(&rest).unwrap_or_default()
    }

    pub fn read_file(&self, path: &str) -> Option<String> {
        let (fsys, _, rest) = self.root_and_path(path);
        let fsys = fsys?;

        let b = fsys.read_file(&rest).ok()?;

        if b.is_empty() {
            return Some(String::new());
        }

        Some(decode_bytes(b))
    }
}

pub fn decode_bytes(mut s: Vec<u8>) -> String {
    if s.len() >= 2 {
        match [s[0], s[1]] {
            [0xFF, 0xFE] => return decode_utf16(&s[2..], false),
            [0xFE, 0xFF] => return decode_utf16(&s[2..], true),
            _ => {}
        }
    }
    if s.len() >= 3 && s[0] == 0xEF && s[1] == 0xBB && s[2] == 0xBF {
        s.drain(..3);
    }

    // Go strings may hold arbitrary bytes; Rust strings must be UTF-8, so
    // invalid sequences are replaced with U+FFFD.
    if simdutf8::basic::from_utf8(&s).is_ok() {
        // SAFETY: simdutf8 validated every byte, and s has not been modified since validation.
        unsafe { String::from_utf8_unchecked(s) }
    } else {
        String::from_utf8_lossy(&s).into_owned()
    }
}

fn decode_utf16(s: &[u8], big_endian: bool) -> String {
    let ints = s.chunks_exact(2).map(|c| {
        if big_endian {
            u16::from_be_bytes([c[0], c[1]])
        } else {
            u16::from_le_bytes([c[0], c[1]])
        }
    });
    char::decode_utf16(ints).map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
}

#[cfg(test)]
mod tests {
    use super::{decode_bytes, valid_path};

    #[test]
    fn decode_utf8_reuses_allocation() {
        for text in [
            "",
            "ascii\0text",
            "café 中文 🎉",
            "\u{FEFF}café 🎉",
            "\u{FEFF}",
        ] {
            let mut bytes = Vec::with_capacity(text.len() + 16);
            bytes.extend_from_slice(text.as_bytes());
            let ptr = bytes.as_ptr();
            let capacity = bytes.capacity();
            let decoded = decode_bytes(bytes);
            assert_eq!(decoded, text.strip_prefix('\u{FEFF}').unwrap_or(text));
            assert_eq!(decoded.as_ptr(), ptr);
            assert_eq!(decoded.capacity(), capacity);
        }
    }

    #[test]
    fn decode_invalid_utf8_replacements() {
        for (bytes, expected) in [
            (&b"\x80"[..], "�"),
            (&b"\xc0\xaf"[..], "��"),
            (&b"\xed\xa0\x80"[..], "���"),
            (&b"\xf4\x90\x80\x80"[..], "����"),
            (&b"a\xe2\x82"[..], "a�"),
            (&b"\xef\xbb\xbf\xf0\x9f\x92"[..], "�"),
        ] {
            assert_eq!(decode_bytes(bytes.to_vec()), expected, "{bytes:?}");
        }
    }

    #[test]
    fn decode_utf8_at_block_boundaries() {
        for prefix_len in [0, 1, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127] {
            for payload in [
                &b""[..],
                &b"\0"[..],
                "é".as_bytes(),
                "中".as_bytes(),
                "🎉".as_bytes(),
                &b"\x80"[..],
                &b"\xc0\xaf"[..],
                &b"\xed\xa0\x80"[..],
                &b"\xf4\x90\x80\x80"[..],
                &b"\xe2\x82"[..],
                &b"\xf0\x9f\x92"[..],
                &b"\xff"[..],
            ] {
                for suffix_len in [0, 64] {
                    let mut bytes = vec![b'a'; prefix_len];
                    bytes.extend_from_slice(payload);
                    bytes.extend(std::iter::repeat_n(b'z', suffix_len));
                    let expected = String::from_utf8_lossy(&bytes).into_owned();
                    assert_eq!(decode_bytes(bytes.clone()), expected, "{bytes:?}");
                    bytes.splice(..0, [0xef, 0xbb, 0xbf]);
                    assert_eq!(decode_bytes(bytes.clone()), expected, "{bytes:?}");
                }
            }
        }
    }

    #[test]
    fn decode_utf16_bom_and_malformed_units() {
        for big_endian in [false, true] {
            let mut bytes = if big_endian {
                vec![0xfe, 0xff]
            } else {
                vec![0xff, 0xfe]
            };
            for unit in [0x0041u16, 0xd83c, 0xdf89, 0xd800, 0x0042, 0xdc00] {
                bytes.extend_from_slice(&if big_endian {
                    unit.to_be_bytes()
                } else {
                    unit.to_le_bytes()
                });
            }
            // As before, an incomplete final UTF-16 code unit is ignored.
            bytes.push(0x61);
            assert_eq!(decode_bytes(bytes), "A🎉�B�");
        }
    }

    #[test]
    fn valid_path_matches_element_split() {
        // Every name of up to 9 bytes over '/', '.', 'a', against fs.ValidPath's element loop.
        fn by_elements(name: &str) -> bool {
            name == "." || name.split('/').all(|elem| !elem.is_empty() && elem != "." && elem != "..")
        }
        let alphabet = [b'/', b'.', b'a'];
        let mut bytes = Vec::new();
        for len in 0..=9 {
            for mut code in 0..3usize.pow(len) {
                bytes.clear();
                for _ in 0..len {
                    bytes.push(alphabet[code % 3]);
                    code /= 3;
                }
                let name = std::str::from_utf8(&bytes).unwrap();
                assert_eq!(valid_path(name), by_elements(name), "{name}");
            }
        }
    }
}
