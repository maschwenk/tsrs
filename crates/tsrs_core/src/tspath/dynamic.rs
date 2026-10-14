use super::*;

pub const DYNAMIC_URI_FILE_NAME_PREFIX: &str = "^/~ts-uri~/";
const DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX: &str = "~ts-uri-escape~";
const DYNAMIC_URI_MODULE_SPECIFIER_ESCAPE_PREFIX: &str = "~ts-uri-spec~";
const DYNAMIC_URI_NO_PATH_ESCAPE_PREFIX: &str = "~ts-uri-no-path~";

pub fn is_encoded_dynamic_file_name(path: &str) -> bool {
    path.starts_with(DYNAMIC_URI_FILE_NAME_PREFIX)
}

pub(crate) fn canonical_dynamic_uri_path(path: &str) -> String {
    if is_encoded_dynamic_file_name(path) && get_root_length(path) == path.len() && !has_trailing_directory_separator(path) {
        format!("{path}/")
    } else {
        path.to_string()
    }
}

pub fn encode_dynamic_uri_path(path: &str) -> String {
    encode_dynamic_uri_path_root_aware(path, true)
}

pub fn encode_dynamic_uri_path_with_suffix(path: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return encode_dynamic_uri_path(path);
    }
    let (before, segment) = match path.rsplit_once('/') {
        Some((before, segment)) => (format!("{}/", encode_dynamic_uri_directory_path(before)), segment),
        None => (String::new(), path),
    };
    let (segment, extension) = split_dynamic_uri_file_extension(segment);
    let payload = format!("{segment}\0{suffix}");
    format!("{before}{DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX}{}~{extension}", encode_hex(payload.as_bytes()))
}

pub fn encode_dynamic_uri_directory_path(path: &str) -> String {
    encode_dynamic_uri_path_root_aware(path, false)
}

fn encode_dynamic_uri_path_root_aware(path: &str, preserve_final_extension: bool) -> String {
    let encoded = encode_dynamic_uri_path_worker(path, preserve_final_extension);
    if !path_is_absolute(&encoded) {
        return encoded;
    }
    match encoded.split_once('/') {
        Some((first, rest)) => format!("{}/{rest}", force_encode_dynamic_uri_path_segment(first, false)),
        None => force_encode_dynamic_uri_path_segment(&encoded, preserve_final_extension),
    }
}

pub fn encode_dynamic_relative_uri_path(path: &str) -> String {
    encode_dynamic_uri_path_root_aware(path, true)
}

pub fn encode_dynamic_relative_uri_directory_path(path: &str) -> String {
    encode_dynamic_uri_path_root_aware(path, false)
}

fn encode_dynamic_uri_path_worker(path: &str, preserve_final_extension: bool) -> String {
    if !path.split('/').any(dynamic_uri_path_segment_needs_encoding) {
        return path.to_string();
    }
    let mut result = String::with_capacity(path.len() + DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX.len());
    let mut remaining = path;
    loop {
        let (segment, rest) = remaining.split_once('/').map_or((remaining, None), |(segment, rest)| (segment, Some(rest)));
        if dynamic_uri_path_segment_needs_encoding(segment) {
            result.push_str(&force_encode_dynamic_uri_path_segment(segment, preserve_final_extension && rest.is_none()));
        } else {
            result.push_str(segment);
        }
        let Some(rest) = rest else { return result };
        result.push('/');
        remaining = rest;
    }
}

pub fn force_encode_dynamic_uri_path_segment(segment: &str, preserve_extension: bool) -> String {
    let (segment, extension) = if preserve_extension && segment != "." && segment != ".." {
        split_dynamic_uri_file_extension(segment)
    } else {
        (segment, String::new())
    };
    format!("{DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX}{}~{extension}", encode_hex(segment.as_bytes()))
}

pub fn encode_dynamic_module_specifier(specifier: &str) -> String {
    encode_dynamic_module_specifier_worker(specifier, true)
}

pub fn encode_dynamic_directory_specifier(specifier: &str) -> String {
    encode_dynamic_module_specifier_worker(specifier, false)
}

fn encode_dynamic_module_specifier_worker(specifier: &str, preserve_final_extension: bool) -> String {
    if !specifier.contains(DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX) && !specifier.contains(DYNAMIC_URI_MODULE_SPECIFIER_ESCAPE_PREFIX) {
        return specifier.to_string();
    }
    let mut result = String::with_capacity(specifier.len() + DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX.len());
    let mut remaining = specifier;
    loop {
        let separator = remaining.find(['/', '\\']);
        let (segment, rest) = separator.map_or((remaining, None), |i| (&remaining[..i], Some(&remaining[i..])));
        result.push_str(&encode_dynamic_module_specifier_segment(segment, preserve_final_extension && rest.is_none()));
        let Some(rest) = rest else { return result };
        result.push(char::from(rest.as_bytes()[0]));
        remaining = &rest[1..];
    }
}

fn encode_dynamic_module_specifier_segment(segment: &str, preserve_extension: bool) -> String {
    if let Some(encoded) = segment.strip_prefix(DYNAMIC_URI_MODULE_SPECIFIER_ESCAPE_PREFIX) {
        let physical = format!("{DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX}{encoded}");
        if decode_dynamic_uri_path_segment(&physical) != physical {
            return physical;
        }
    }
    if segment.starts_with(DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX) {
        return force_encode_dynamic_uri_path_segment(segment, preserve_extension);
    }
    segment.to_string()
}

pub fn dynamic_uri_path_to_module_specifier(path: &str) -> String {
    if !path.contains(DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX) {
        return path.to_string();
    }
    path.split('/').map(|segment| {
        if segment.starts_with(DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX) && decode_dynamic_uri_path_segment(segment) != segment {
            format!("{DYNAMIC_URI_MODULE_SPECIFIER_ESCAPE_PREFIX}{}", segment.trim_start_matches(DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX))
        } else {
            segment.to_string()
        }
    }).collect::<Vec<_>>().join("/")
}

pub fn encode_dynamic_logical_module_specifier(specifier: &str) -> String {
    if specifier.is_empty() {
        return String::new();
    }
    let trailing_separator = has_trailing_directory_separator(specifier);
    let specifier = if trailing_separator { remove_trailing_directory_separator(specifier) } else { specifier };
    let mut encoded = dynamic_uri_path_to_module_specifier(&encode_dynamic_relative_uri_path(specifier));
    if trailing_separator {
        encoded.push('/');
    }
    encoded
}

fn dynamic_uri_path_segment_needs_encoding(segment: &str) -> bool {
    segment.is_empty() || segment == "." || segment == ".."
        || segment.starts_with(DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX)
        || segment.starts_with(DYNAMIC_URI_MODULE_SPECIFIER_ESCAPE_PREFIX)
        || segment.starts_with(DYNAMIC_URI_NO_PATH_ESCAPE_PREFIX)
        || segment.contains('\\')
}

pub fn encode_dynamic_uri_no_path(suffix: &str) -> String {
    format!("{DYNAMIC_URI_NO_PATH_ESCAPE_PREFIX}{}~", encode_hex(suffix.as_bytes()))
}

pub fn decode_dynamic_uri_no_path(path: &str) -> Option<String> {
    let encoded = path.strip_prefix(DYNAMIC_URI_NO_PATH_ESCAPE_PREFIX)?.strip_suffix('~')?;
    decode_hex(encoded)
}

fn split_dynamic_uri_file_extension(segment: &str) -> (&str, String) {
    let base_name = segment.rsplit('\\').next().unwrap();
    let mut extension = get_declaration_file_extension(base_name);
    if extension.is_empty() {
        extension = get_any_extension_from_path(base_name, &[], true);
    }
    (&segment[..segment.len() - extension.len()], extension)
}

pub fn decode_dynamic_uri_path(path: &str) -> String {
    if !path.contains(DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX) {
        return path.to_string();
    }
    path.split('/').map(decode_dynamic_uri_path_segment).collect::<Vec<_>>().join("/")
}

pub fn try_decode_dynamic_uri_path(path: &str) -> Option<String> {
    path.split('/').map(try_decode_dynamic_uri_path_segment).collect::<Option<Vec<_>>>().map(|segments| segments.join("/"))
}

pub fn decode_dynamic_uri_path_for_disk(path: &str) -> Option<String> {
    let decoded = decode_dynamic_uri_path(path);
    if path_is_absolute(&decoded) {
        return None;
    }
    let mut segments = decoded.split('/').peekable();
    while let Some(segment) = segments.next() {
        if segment.is_empty() {
            return segments.peek().is_none().then_some(decoded);
        }
        if segment == "." || segment == ".." || segment.contains('\\') {
            return None;
        }
    }
    Some(decoded)
}

pub fn decode_dynamic_uri_path_segment(segment: &str) -> String {
    try_decode_dynamic_uri_path_segment(segment).unwrap_or_else(|| segment.to_string())
}

pub fn try_decode_dynamic_uri_path_segment(segment: &str) -> Option<String> {
    let Some(encoded) = segment.strip_prefix(DYNAMIC_URI_PATH_SEGMENT_ESCAPE_PREFIX) else {
        return (!dynamic_uri_path_segment_needs_encoding(segment)).then(|| segment.to_string());
    };
    let (encoded, extension) = encoded.split_once('~')?;
    let decoded = decode_hex(encoded)?;
    match decoded.split_once('\0') {
        Some((base, suffix)) => Some(format!("{base}{extension}{suffix}")),
        None => Some(format!("{decoded}{extension}")),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    result
}

fn decode_hex(encoded: &str) -> Option<String> {
    if !encoded.len().is_multiple_of(2) {
        return None;
    }
    let bytes = encoded.as_bytes().as_chunks::<2>().0.iter().map(|pair| {
        Some((char::from(pair[0]).to_digit(16)? * 16 + char::from(pair[1]).to_digit(16)?) as u8)
    }).collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}
