// Package contentmappertest provides realistic content mapper implementations used by tests.

use std::fmt::Write as _;
use std::sync::Arc;

use tsrs_contentmapper::{
    unmarshal, CloseProjectParams, InitializeResult, MappedOutput, OpenProjectParams, OpenProjectResult, OptionDiagnosticResult,
    PositionEncoding, ProtocolJson, METHOD_CLOSE_PROJECT, METHOD_OPEN_PROJECT,
};
use tsrs_core::json::{self, Value};
use tsrs_ipc::{self as ipc, Handler};
use tsrs_spanmap::{Feature, Kind, Segment};

// protocol.go:14 noNotifications, which every handler of the package embeds for its HandleNotification.
pub(crate) fn no_notifications() -> Result<(), ipc::Error> {
    Ok(())
}

// protocol.go:20
pub(crate) fn initialize_result(source: &str) -> InitializeResult {
    InitializeResult { position_encoding: PositionEncoding::UTF8, diagnostic_source: source.to_string() }
}

// protocol.go:27
pub(crate) fn identity_mapped_output(content: &str) -> Result<MappedOutput, ipc::Error> {
    let mappings = marshal_span_map(&[Segment {
        virtual_end: content.len() as i32,
        original_end: content.len() as i32,
        kind: Kind::Verbatim,
        features: Feature::All,
        ..Default::default()
    }])?;
    Ok(MappedOutput { text: content.to_string(), extension: ".ts".to_string(), mappings: Some(mappings), ..Default::default() })
}

// protocol.go:40 (Go embeds the ipc.Handler it wraps.)
pub(crate) struct staticProjectHandler {
    pub(crate) handler: Arc<dyn testHandler>,
}

// protocol.go:42
pub(crate) trait projectLifecycleHandler {
    fn open_project(&self, params: OpenProjectParams) -> Result<(), ipc::Error>;
    fn close_project(&self, params: CloseProjectParams);
}

// An ipc.Handler of this package. staticProjectHandler asks the handler it wraps whether it is also a
// projectLifecycleHandler with a type assertion (`h.Handler.(projectLifecycleHandler)`); `project_lifecycle` is that
// assertion.
pub(crate) trait testHandler: Handler {
    fn project_lifecycle(&self) -> Option<&dyn projectLifecycleHandler> {
        None
    }
}

impl Handler for staticProjectHandler {
    // protocol.go:47
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_OPEN_PROJECT => {
                let p: OpenProjectParams = unmarshal_params(params)?;
                let options = raw_text(p.options.as_ref());
                if let Some(handler) = self.handler.project_lifecycle() {
                    handler.open_project(p)?;
                }
                let mut diagnostics = Vec::new();
                if options == r#"{"plugins":[{"name":1}]}"# {
                    diagnostics = vec![OptionDiagnosticResult {
                        path: vec![Value::String("plugins".to_string()), Value::Number(0.0), Value::String("name".to_string())],
                        message_text: "Option 'name' requires a string.".to_string(),
                        code: 123,
                    }];
                }
                Ok(OpenProjectResult { option_diagnostics: diagnostics, ..Default::default() }.marshal_json())
            }
            METHOD_CLOSE_PROJECT => {
                let p: CloseProjectParams = unmarshal_params(params)?;
                if let Some(handler) = self.handler.project_lifecycle() {
                    handler.close_project(p);
                }
                Ok(Value::Null)
            }
            _ => self.handler.handle_request(method, params),
        }
    }

    fn handle_notification(&self, method: &str, params: Option<&Value>) -> Result<(), ipc::Error> {
        self.handler.handle_notification(method, params)
    }
}

// Go's `json.Unmarshal(params, &p)` in a handler: the error becomes the handler's error response.
pub(crate) fn unmarshal_params<T: ProtocolJson>(params: Option<&Value>) -> Result<T, ipc::Error> {
    unmarshal(params.cloned()).map_err(ipc::Error::new)
}

// Go's `json.Value(spanmap.New(segments).Marshal())`: the span map's tuple-array JSON, as the value it is sent as.
pub(crate) fn marshal_span_map(segments: &[Segment]) -> Result<Value, ipc::Error> {
    let data = tsrs_spanmap::new(segments).marshal().map_err(ipc::Error::new)?;
    let text = String::from_utf8(data).map_err(|err| ipc::Error::new(err.to_string()))?;
    json::unmarshal(&text).map_err(ipc::Error::new)
}

// Go's `string(raw)` of a json.Value: "" when it is empty (absent), else its compact JSON text.
pub(crate) fn raw_text(raw: Option<&Value>) -> String {
    raw.map(|value| json::marshal(value).unwrap_or_default()).unwrap_or_default()
}

// The handlers' error for a method they do not implement: Go's `fmt.Errorf("contentmappertest: unexpected method %q", method)`.
pub(crate) fn unexpected_method(method: &str) -> ipc::Error {
    ipc::Error::new(format!("contentmappertest: unexpected method {}", quote(method)))
}

// Go `strconv.Quote`, which fmt's %q verb uses for a string.
pub(crate) fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
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
            c if (c as u32) < 0x20 || c == '\x7f' => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c if c.is_control() || (c.is_whitespace() && c != ' ') => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
