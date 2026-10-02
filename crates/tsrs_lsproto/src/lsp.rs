use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::{Mutex, OnceLock};

use rustc_hash::FxHashSet;
use tsrs_core::tspath;

use crate::error::Error;
use crate::json::{is_null, kind, kind_string, IsZero, Json, JsonError, JsonKey, Value};
use crate::jsonrpc::ID;
use crate::{CodeActionKind, ErrorCode, Location, MarkupKind, Position, RequestMessage};

// lsp.go:17
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocumentUri(pub String); // !!!

impl DocumentUri {
    // lsp.go:19
    pub fn file_name(&self) -> String {
        let uri = self.0.as_str();
        if tsrs_vfs::bundled::is_bundled(uri) {
            return uri.to_string();
        }
        if uri.starts_with("file://") {
            let Ok((host, path)) = parse_file_url(uri) else {
                panic!("invalid file URI: {}", uri);
            };
            if !host.is_empty() {
                return format!("//{}{}", host, path);
            }
            return fix_windows_uri_path(&path).to_string();
        }

        // Leave all other URIs escaped so we can round-trip them.

        let Some((scheme, path)) = uri.split_once(':') else {
            panic!("invalid URI: {}", uri);
        };

        let mut authority = "ts-nul-authority";
        let mut path = path;
        if let Some(rest) = path.strip_prefix("//") {
            let Some((a, p)) = rest.split_once('/') else {
                panic!("invalid URI: {}", uri);
            };
            authority = a;
            path = p;
        }

        format!("^/{}/{}/{}", scheme, authority, path)
    }

    // lsp.go:52
    pub fn path(&self, use_case_sensitive_file_names: bool) -> tspath::Path {
        let file_name = self.file_name();
        tspath::to_path(&file_name, "", use_case_sensitive_file_names)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for DocumentUri {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DocumentUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for DocumentUri {
    fn from(s: &str) -> DocumentUri {
        DocumentUri(s.to_string())
    }
}

impl From<String> for DocumentUri {
    fn from(s: String) -> DocumentUri {
        DocumentUri(s)
    }
}

impl Json for DocumentUri {
    const GO_TYPE: &'static str = "lsproto.DocumentUri";

    fn to_json(&self) -> Value {
        Value::String(self.0.clone())
    }

    fn from_json(v: &Value) -> Result<Self, JsonError> {
        crate::json::decode_string(v, Self::GO_TYPE).map(|s| DocumentUri(s.to_string()))
    }
}

impl JsonKey for DocumentUri {
    const GO_KEY_TYPE: &'static str = "lsproto.DocumentUri";

    fn key_string(&self) -> &str {
        &self.0
    }

    fn from_key(s: &str) -> Self {
        DocumentUri(s.to_string())
    }
}

impl IsZero for DocumentUri {
    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }
}

// Go `url.Parse` as far as `FileName` uses it: the host and the unescaped path of a `file://` URI.
fn parse_file_url(raw_url: &str) -> Result<(String, String), String> {
    let (u, frag) = raw_url.split_once('#').unwrap_or((raw_url, ""));
    if u.bytes().any(|b| b < b' ' || b == 0x7f) {
        return Err("net/url: invalid control character in URL".to_string());
    }
    // getScheme: "file"
    let mut rest = &u["file:".len()..];
    if rest.ends_with('?') && rest.matches('?').count() == 1 {
        rest = &rest[..rest.len() - 1];
    } else {
        rest = rest.split_once('?').map_or(rest, |(r, _)| r);
    }
    let mut host = String::new();
    if let Some(after) = rest.strip_prefix("//") {
        let (authority, path_rest) = match after.find('/') {
            Some(i) => (&after[..i], &after[i..]),
            None => (after, ""),
        };
        host = parse_authority(authority)?;
        rest = path_rest;
    }
    let path = unescape(rest, Encoding::Path)?;
    if !frag.is_empty() {
        unescape(frag, Encoding::Fragment)?;
    }
    Ok((host, path))
}

fn parse_authority(authority: &str) -> Result<String, String> {
    let i = authority.rfind('@');
    let host = match i {
        None => parse_host(authority)?,
        Some(i) => parse_host(&authority[i + 1..])?,
    };
    let Some(i) = i else {
        return Ok(host);
    };
    let userinfo = &authority[..i];
    let valid_userinfo = userinfo.chars().all(|r| {
        r.is_ascii_alphanumeric()
            || matches!(r, '-' | '.' | '_' | ':' | '~' | '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '=' | '%' | '@')
    });
    if !valid_userinfo {
        return Err("net/url: invalid userinfo".to_string());
    }
    match userinfo.split_once(':') {
        None => {
            unescape(userinfo, Encoding::UserPassword)?;
        }
        Some((username, password)) => {
            unescape(username, Encoding::UserPassword)?;
            unescape(password, Encoding::UserPassword)?;
        }
    }
    Ok(host)
}

fn valid_optional_port(port: &str) -> bool {
    if port.is_empty() {
        return true;
    }
    if !port.starts_with(':') {
        return false;
    }
    port[1..].bytes().all(|b| b.is_ascii_digit())
}

fn parse_host(host: &str) -> Result<String, String> {
    match host.rfind('[') {
        Some(i) if i > 0 => Err("invalid IP-literal".to_string()),
        Some(_) => {
            let Some(close) = host.rfind(']') else {
                return Err("missing ']' in host".to_string());
            };
            let colon_port = &host[close + 1..];
            if !valid_optional_port(colon_port) {
                return Err(format!("invalid port {} after host", go_quote(colon_port.as_bytes())));
            }
            let unescaped_colon_port = unescape(colon_port, Encoding::Host)?;
            let hostname = &host[1..close];
            let unescaped_hostname = match hostname.find("%25") {
                Some(zone) => unescape(&hostname[..zone], Encoding::Host)? + &unescape(&hostname[zone..], Encoding::Zone)?,
                None => unescape(hostname, Encoding::Host)?,
            };
            Ok(format!("[{}]{}", unescaped_hostname, unescaped_colon_port))
        }
        None => {
            if let Some(i) = host.rfind(':') {
                let colon_port = &host[i..];
                if !valid_optional_port(colon_port) {
                    return Err(format!("invalid port {} after host", go_quote(colon_port.as_bytes())));
                }
            }
            unescape(host, Encoding::Host)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Path,
    UserPassword,
    Host,
    Zone,
    Fragment,
}

fn should_escape_host(c: u8) -> bool {
    !(c.is_ascii_alphanumeric()
        || matches!(
            c,
            b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b':' | b'[' | b']' | b'<' | b'>' | b'"' | b'-' | b'_' | b'.' | b'~'
        ))
}

fn unhex(c: u8) -> u8 {
    (c as char).to_digit(16).unwrap() as u8
}

// Go net/url `unescape`.
fn unescape(s: &str, mode: Encoding) -> Result<String, String> {
    let b = s.as_bytes();
    let mut n = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                n += 1;
                if i + 2 >= b.len() || !b[i + 1].is_ascii_hexdigit() || !b[i + 2].is_ascii_hexdigit() {
                    let end = (i + 3).min(b.len());
                    return Err(format!("invalid URL escape {}", go_quote(&b[i..end])));
                }
                if mode == Encoding::Host && unhex(b[i + 1]) < 8 && &b[i..i + 3] != b"%25" {
                    return Err(format!("invalid URL escape {}", go_quote(&b[i..i + 3])));
                }
                if mode == Encoding::Zone {
                    let v = unhex(b[i + 1]) << 4 | unhex(b[i + 2]);
                    if &b[i..i + 3] != b"%25" && v != b' ' && should_escape_host(v) {
                        return Err(format!("invalid URL escape {}", go_quote(&b[i..i + 3])));
                    }
                }
                i += 3;
            }
            c => {
                if (mode == Encoding::Host || mode == Encoding::Zone) && c < 0x80 && should_escape_host(c) {
                    return Err(format!("invalid character {} in host name", go_quote(&b[i..i + 1])));
                }
                i += 1;
            }
        }
    }
    if n == 0 {
        return Ok(s.to_string());
    }
    let mut t = Vec::with_capacity(b.len() - 2 * n);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            t.push(unhex(b[i + 1]) << 4 | unhex(b[i + 2]));
            i += 3;
        } else {
            t.push(b[i]);
            i += 1;
        }
    }
    Ok(String::from_utf8_lossy(&t).into_owned())
}

// Go `strconv.Quote` (as `%q` formats a string or byte slice).
pub(crate) fn go_quote(b: &[u8]) -> String {
    let mut out = String::from("\"");
    let s = String::from_utf8_lossy(b);
    for c in s.chars() {
        match c {
            '\x07' => out.push_str("\\a"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x0b' => out.push_str("\\v"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// lsp.go:57
fn fix_windows_uri_path(path: &str) -> &str {
    if let Some(rest) = path.strip_prefix('/') {
        let b = rest.as_bytes();
        if b.len() >= 2 && tspath::is_volume_character(b[0]) && b[1] == b':' {
            return rest;
        }
    }
    path
}

// lsp.go:66
pub trait HasTextDocumentURI {
    fn text_document_uri(&self) -> &DocumentUri;
}

// lsp.go:70
pub trait HasTextDocumentPosition: HasTextDocumentURI {
    fn text_document_position(&self) -> Position;
}

// lsp.go:75
pub trait HasLocations {
    fn get_locations(&self) -> Option<&Vec<Location>>;
}

// lsp.go:79
pub trait HasLocation {
    fn get_location(&self) -> Location;
}

// lsp.go:83
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct URI(pub String); // !!!

impl Deref for URI {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for URI {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Json for URI {
    const GO_TYPE: &'static str = "lsproto.URI";

    fn to_json(&self) -> Value {
        Value::String(self.0.clone())
    }

    fn from_json(v: &Value) -> Result<Self, JsonError> {
        crate::json::decode_string(v, Self::GO_TYPE).map(|s| URI(s.to_string()))
    }
}

impl IsZero for URI {
    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }
}

// lsp.go:85. Method names and string enumeration values are open sets of short strings; they are
// interned so that the types stay `Copy` and the known values can be matched as constants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Method(pub &'static str);

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Json for Method {
    const GO_TYPE: &'static str = "lsproto.Method";

    fn to_json(&self) -> Value {
        Value::String(self.0.to_string())
    }

    fn from_json(v: &Value) -> Result<Self, JsonError> {
        crate::json::decode_string(v, Self::GO_TYPE).map(Method::from_str)
    }
}

impl IsZero for Method {
    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }
}

// The process-lifetime string for `s` (Go converts a decoded string to a named string type).
pub fn intern(s: &str) -> &'static str {
    if s.is_empty() {
        return "";
    }
    static TABLE: OnceLock<Mutex<FxHashSet<&'static str>>> = OnceLock::new();
    let mut table = TABLE.get_or_init(Default::default).lock().unwrap();
    if let Some(interned) = table.get(s) {
        return interned;
    }
    let interned: &'static str = Box::leak(s.to_string().into_boxed_str());
    table.insert(interned);
    interned
}

// lsp.go:87
pub(crate) fn err_not_object(k: u8) -> String {
    format!("expected object start, but encountered {}", kind_string(k))
}

// lsp.go:91
pub(crate) fn err_null(field: &str) -> String {
    format!("null value is not allowed for field {}", go_quote(field.as_bytes()))
}

// lsp.go:95
pub(crate) fn err_missing(props: &[&str]) -> String {
    format!("missing required properties: {}", props.join(", "))
}

// lsp.go:99
pub(crate) fn err_invalid_kind(type_name: &str, got: u8) -> String {
    format!("invalid {}: got {}", type_name, kind_string(got))
}

// lsp.go:103
pub(crate) fn err_invalid_value(type_name: &str, data: &Value) -> String {
    format!("invalid {}: {}", type_name, tsrs_core::json::marshal(data).unwrap_or_default())
}

// lsp.go:107
pub(crate) fn err_literal_mismatch(type_name: &str, expected: &str, got: &Value) -> String {
    format!("expected {} value {}, got {}", type_name, expected, tsrs_core::json::marshal(got).unwrap_or_default())
}

// lsp.go:111
pub(crate) fn assert_only_one(message: &str, count: usize) {
    if count != 1 {
        panic!("{}", message);
    }
}

// lsp.go:117
pub(crate) fn assert_at_most_one(message: &str, count: usize) {
    if count > 1 {
        panic!("{}", message);
    }
}

// lsp.go:131: the value of a top-level field of a JSON object; None if `data` is not an object or has
// no such field. (Go's jsonKeyCheck compares raw key tokens; decoded keys are compared directly.)
pub(crate) fn json_object_raw_field<'a>(data: &'a Value, field: &str) -> Option<&'a Value> {
    let Value::Object(members) = data else {
        return None;
    };
    members.get(field)
}

// lsp.go:161: the index in `keys` of the first top-level key of the object (in document order) that is
// one of `keys`, or -1.
pub(crate) fn json_object_has_key(data: &Value, keys: &[&str]) -> i32 {
    let Value::Object(members) = data else {
        return -1;
    };
    for name in members.keys() {
        for (i, key) in keys.iter().enumerate() {
            if name == key {
                return i as i32;
            }
        }
    }
    -1
}

// Inspired by https://www.youtube.com/watch?v=dab3I-HcTVk

// lsp.go:188
pub struct RequestInfo<Params, Resp> {
    _marker: PhantomData<fn() -> (Params, Resp)>,
    pub method: Method,
}

impl<Params, Resp> Clone for RequestInfo<Params, Resp> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Params, Resp> Copy for RequestInfo<Params, Resp> {}

impl<Params, Resp> RequestInfo<Params, Resp> {
    pub const fn new(method: Method) -> Self {
        RequestInfo { _marker: PhantomData, method }
    }
}

impl<Params: Json, Resp: Json> RequestInfo<Params, Resp> {
    // lsp.go:194. `result` is a response's raw result (None when the response had none).
    pub fn unmarshal_result(&self, result: Option<&Value>) -> Result<Resp, JsonError> {
        let Some(raw) = result else {
            return Err(JsonError::plain("unexpected EOF"));
        };
        Resp::from_json(raw)
    }

    // lsp.go:207
    pub fn new_request_message(&self, id: Option<ID>, params: Params) -> RequestMessage {
        RequestMessage { id, method: self.method, params: Some(params.to_json()) }
    }
}

// lsp.go:215
pub struct NotificationInfo<Params> {
    _marker: PhantomData<fn() -> Params>,
    pub method: Method,
}

impl<Params> Clone for NotificationInfo<Params> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Params> Copy for NotificationInfo<Params> {}

impl<Params> NotificationInfo<Params> {
    pub const fn new(method: Method) -> Self {
        NotificationInfo { _marker: PhantomData, method }
    }
}

impl<Params: Json> NotificationInfo<Params> {
    // lsp.go:220
    pub fn new_notification_message(&self, params: Params) -> RequestMessage {
        RequestMessage { id: None, method: self.method, params: Some(params.to_json()) }
    }
}

impl RequestMessage {
    // lsp.go:235
    // UnmarshalParams decodes the params of an inbound request or notification
    // message into the requested type. Inbound messages store their params as a
    // raw value (see Message's decoding); decoding is deferred to the point of
    // dispatch.
    //
    // A NoParams method must be given no params; every other method must be given
    // params as an object or array. A violation returns ErrorCodeInvalidParams.
    pub fn unmarshal_params<T: Json + Default>(&self) -> Result<T, Error> {
        let raw = self.params.as_ref();

        // Whether the method was declared with NoParams.
        if T::IS_NO_PARAMS {
            if let Some(raw) = raw {
                return Err(Error::wrap_code(
                    ErrorCode::InvalidParams,
                    Error::new(format!("expected no params, got {}", tsrs_core::json::marshal(raw).unwrap_or_default())),
                ));
            }
            return Ok(T::default());
        }

        // The base protocol defines params as `array | object`; reject anything else
        // (absent, null, or a scalar).
        let raw = match raw {
            Some(raw @ (Value::Object(_) | Value::Array(_))) => raw,
            _ => {
                return Err(Error::wrap_code(ErrorCode::InvalidParams, Error::new("params must be an object or array")));
            }
        };
        T::from_json(raw).map_err(|err| Error::wrap_code(ErrorCode::InvalidParams, err))
    }
}

// lsp.go:266
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Null;

impl Json for Null {
    const GO_TYPE: &'static str = "lsproto.Null";

    // lsp.go:279
    fn to_json(&self) -> Value {
        Value::Null
    }

    // lsp.go:268
    fn from_json(v: &Value) -> Result<Self, JsonError> {
        if !is_null(v) {
            return Err(JsonError::method(
                Self::GO_TYPE,
                format!("expected null, got {}", tsrs_core::json::marshal(v).unwrap_or_default()),
            ));
        }
        Ok(Null)
    }
}

// lsp.go:283
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NoParams;

impl Json for NoParams {
    const GO_TYPE: &'static str = "lsproto.NoParams";
    const IS_NO_PARAMS: bool = true;

    fn to_json(&self) -> Value {
        Value::Object(Default::default())
    }

    fn from_json(v: &Value) -> Result<Self, JsonError> {
        crate::structcodec::struct_members(v, Self::GO_TYPE, false)?;
        Ok(NoParams)
    }
}

struct clientCapabilitiesKey(std::sync::Arc<crate::ResolvedClientCapabilities>);

// lsp.go:289
pub fn with_client_capabilities(ctx: &tsrs_core::context::Context, caps: std::sync::Arc<crate::ResolvedClientCapabilities>) -> tsrs_core::context::Context {
    ctx.with_value(clientCapabilitiesKey(caps))
}

// lsp.go:293
pub fn get_client_capabilities(ctx: &tsrs_core::context::Context) -> std::sync::Arc<crate::ResolvedClientCapabilities> {
    if let Some(caps) = ctx.value::<clientCapabilitiesKey>() {
        return caps.0.clone();
    }
    std::sync::Arc::new(crate::ResolvedClientCapabilities::default())
}

// lsp.go:285
impl IsZero for NoParams {
    fn is_zero(&self) -> bool {
        true
    }
}

// Go's anonymous `struct{}` (the `{}` literal type of the meta model).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EmptyObject;

impl Json for EmptyObject {
    const GO_TYPE: &'static str = "struct {}";

    fn to_json(&self) -> Value {
        Value::Object(Default::default())
    }

    fn from_json(v: &Value) -> Result<Self, JsonError> {
        crate::structcodec::struct_members(v, Self::GO_TYPE, false)?;
        Ok(EmptyObject)
    }
}

// lsp.go:302
// PreferredMarkupKind returns the first (most preferred) markup kind from the given formats,
// or MarkupKindPlainText if the slice is empty.
pub fn preferred_markup_kind(formats: &[MarkupKind]) -> MarkupKind {
    if !formats.is_empty() {
        return formats[0];
    }
    MarkupKind::PlainText
}

impl CodeActionKind {
    // lsp.go:310
    // Contains reports whether other is this code action kind or one of its children.
    pub fn contains(self, other: CodeActionKind) -> bool {
        self == other || self == CodeActionKind::Empty || other.0.strip_prefix(self.0).is_some_and(|rest| rest.starts_with('.'))
    }

    // lsp.go:316
    pub const SourceFixAllTs: CodeActionKind = CodeActionKind("source.fixAll.ts");
    pub const SourceOrganizeImportsTs: CodeActionKind = CodeActionKind("source.organizeImports.ts");
    pub const SourceRemoveUnusedImportsTs: CodeActionKind = CodeActionKind("source.removeUnusedImports.ts");
    pub const SourceSortImportsTs: CodeActionKind = CodeActionKind("source.sortImports.ts");
}
