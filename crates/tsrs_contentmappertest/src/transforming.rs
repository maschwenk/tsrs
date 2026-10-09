use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rustc_hash::FxHashMap;
use tsrs_contentmapper::{
    CloseProjectParams, Diagnostic, DiagnosticDirectivePolicy, DiagnosticDirectives, MappedDiagnosticDirective, MappedOutput,
    OpenProjectParams, ProtocolJson, TransformParams, TransformResult, UnusedExpectDirectiveDiagnostic, METHOD_INITIALIZE,
    METHOD_TRANSFORM,
};
use tsrs_core::collections::OrderedMap;
use tsrs_core::json::{self, Value};
use tsrs_core::TextRange;
use tsrs_ipc as ipc;
use tsrs_spanmap::{Feature, Kind, Segment, SpanMap};

use crate::protocol::{
    initialize_result, marshal_span_map, no_notifications, projectLifecycleHandler, quote, testHandler, unexpected_method,
    unmarshal_params,
};

// transforming.go:17
const PREAMBLE: &str = "const __VERSION = \"1.0.0\";\n";

// transforming.go:19
pub static DECLARED_OPTIONS: [&str; 2] = ["target", "jsx"];

// transforming.go:21
const DIAGNOSTIC_SOURCE: &str = "box";
const UNCLOSED_INTERPOLATION_CODE: i32 = 1000;

// Handler implements the transforming content mapper protocol.
// transforming.go:27 (A project's options are None when its compilerOptions were JSON null, Go's nil map.)
#[derive(Default)]
pub struct Handler {
    mu: Mutex<FxHashMap<String, Option<Arc<OrderedMap<String, Value>>>>>,
}

impl projectLifecycleHandler for Handler {
    // transforming.go:35
    fn open_project(&self, p: OpenProjectParams) -> Result<(), ipc::Error> {
        // json.Unmarshal(p.CompilerOptions, &options) into a *collections.OrderedMap.
        let options = match p.compiler_options {
            None => return Err(ipc::Error::new("unexpected EOF")),
            Some(Value::Null) => None,
            Some(Value::Object(members)) => Some(Arc::new(members)),
            Some(_) => return Err(ipc::Error::new("cannot unmarshal non-object JSON value into Map")),
        };
        lock(&self.mu).insert(p.project_handle, options);
        Ok(())
    }

    // transforming.go:49
    fn close_project(&self, p: CloseProjectParams) {
        lock(&self.mu).remove(&p.project_handle);
    }
}

impl testHandler for Handler {
    fn project_lifecycle(&self) -> Option<&dyn projectLifecycleHandler> {
        Some(self)
    }
}

impl ipc::Handler for Handler {
    // transforming.go:55
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result(DIAGNOSTIC_SOURCE).marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                let options = lock(&self.mu).get(&p.project_handle).cloned().flatten();
                let Some(options) = options else {
                    return Err(ipc::Error::new(format!("contentmappertest: project {} is not open", quote(&p.project_handle))));
                };
                let (text, mappings, diagnostics, diagnostic_directives) = transform(&p.content, Some(&options))?;
                Ok(TransformResult {
                    mapped_output: MappedOutput {
                        text,
                        extension: mapped_extension(&p.content),
                        mappings: Some(mappings),
                        diagnostic_directives,
                    },
                    diagnostics,
                    ..Default::default()
                }
                .marshal_json())
            }
            _ => Err(unexpected_method(method)),
        }
    }

    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        no_notifications()
    }
}

// transforming.go:83
fn mapped_extension(content: &str) -> String {
    const PREFIX: &str = "// @box-extension:";
    if let Some((first_line, _)) = content.split_once('\n') {
        if let Some(extension) = first_line.strip_prefix(PREFIX) {
            return extension.trim().to_string();
        }
    }
    ".ts".to_string()
}

type transformOutput = (String, Value, Vec<Diagnostic>, Option<DiagnosticDirectives>);

// transforming.go:91 (Go's writeVerbatim and writeAtom closures share the builder and the segments, which
// `virtualWriter` holds.)
fn transform(content: &str, options: Option<&OrderedMap<String, Value>>) -> Result<transformOutput, ipc::Error> {
    let mut w = virtualWriter { virtual_: String::new(), segments: Vec::new() };
    let mut diagnostics = Vec::new();

    w.virtual_.push_str(PREAMBLE);

    let mut pos = 0;
    while pos < content.len() {
        let Some(rel) = content[pos..].find("#{") else {
            w.write_verbatim(content, pos, content.len());
            break;
        };
        let token_start = pos + rel;

        let line_end = match content[token_start..].find('\n') {
            Some(rel) => token_start + rel,
            None => content.len(),
        };
        let Some(close_rel) = content[token_start..line_end].find('}') else {
            w.write_verbatim(content, pos, token_start);
            w.write_atom("undefined", token_start, line_end);
            diagnostics.push(Diagnostic {
                message_text: "Unclosed interpolation.".to_string(),
                start: token_start as i64,
                length: (line_end - token_start) as i64,
                code: UNCLOSED_INTERPOLATION_CODE,
            });
            pos = line_end;
            continue;
        };
        let token_end = token_start + close_rel + 1;
        let name = &content[token_start + "#{".len()..token_end - "}".len()];

        w.write_verbatim(content, pos, token_start);
        w.write_atom(&render_option(options, name), token_start, token_end);
        pos = token_end;
    }

    let span_map = tsrs_spanmap::new(&w.segments);
    let mappings = marshal_span_map(&span_map.segments())?;
    let directives = diagnostic_directives(content, &span_map);
    Ok((w.virtual_, mappings, diagnostics, directives))
}

struct virtualWriter {
    virtual_: String,
    segments: Vec<Segment>,
}

impl virtualWriter {
    // transforming.go:98
    fn write_verbatim(&mut self, content: &str, from: usize, to: usize) {
        if to <= from {
            return;
        }
        let virtual_start = self.virtual_.len() as i32;
        self.virtual_.push_str(&content[from..to]);
        self.segments.push(Segment {
            virtual_start,
            virtual_end: self.virtual_.len() as i32,
            original_start: from as i32,
            original_end: to as i32,
            kind: Kind::Verbatim,
            features: Feature::All,
        });
    }

    // transforming.go:114
    fn write_atom(&mut self, value: &str, from: usize, to: usize) {
        let virtual_start = self.virtual_.len() as i32;
        self.virtual_.push_str(value);
        self.segments.push(Segment {
            virtual_start,
            virtual_end: self.virtual_.len() as i32,
            original_start: from as i32,
            original_end: to as i32,
            kind: Kind::Atom,
            features: Feature::All,
        });
    }
}

// transforming.go:169
fn diagnostic_directives(content: &str, mappings: &SpanMap) -> Option<DiagnosticDirectives> {
    let wrap = |directives: Vec<MappedDiagnosticDirective>, unused: Vec<UnusedExpectDirectiveDiagnostic>| {
        Some(DiagnosticDirectives { unused_expect_directive_diagnostics: unused, directives })
    };
    const INVALID_PREFIX: &str = "// @box-invalid-directive:";
    if let Some(rest) = content.strip_prefix(INVALID_PREFIX) {
        let directive = |virtual_start: i64, virtual_end: i64, original_start: i64, policy: DiagnosticDirectivePolicy| {
            MappedDiagnosticDirective { original_start, virtual_start, virtual_end, policy, ..Default::default() }
        };
        let ignore = DiagnosticDirectivePolicy::Ignore;
        let expect = DiagnosticDirectivePolicy::Expect;
        match rest.split('\n').next().unwrap_or_default().trim() {
            "invalid-range" => return wrap(vec![directive(-1, 0, 0, ignore)], Vec::new()),
            "original-range-out-of-bounds" => return wrap(vec![directive(0, 0, content.len() as i64 + 1, ignore)], Vec::new()),
            "virtual-range-out-of-bounds" => return wrap(vec![directive(1 << 20, 1 << 20, 0, ignore)], Vec::new()),
            "invalid-policy" => return wrap(vec![directive(0, 0, 0, DiagnosticDirectivePolicy(2))], Vec::new()),
            "ignore-with-unused-diagnostic" => {
                return wrap(vec![directive(0, 0, 0, ignore)], vec![UnusedExpectDirectiveDiagnostic::default()]);
            }
            "expect-without-unused-diagnostic" => return wrap(vec![directive(0, 0, 0, expect)], Vec::new()),
            "invalid-unused-diagnostic-index" => {
                let directive = MappedDiagnosticDirective { unused_expect_directive_index: Some(1), ..directive(0, 0, 0, expect) };
                return wrap(vec![directive], vec![UnusedExpectDirectiveDiagnostic::default()]);
            }
            "overlap" => return wrap(vec![directive(0, 2, 0, ignore), directive(1, 3, 0, ignore)], Vec::new()),
            _ => {}
        }
    }
    const IGNORE_PREFIX: &str = "// @box-ignore";
    const EXPECT_PREFIX: &str = "// @box-expect-error:";
    let mut result: Vec<MappedDiagnosticDirective> = Vec::new();
    let mut unused_diagnostics: Vec<UnusedExpectDirectiveDiagnostic> = Vec::new();
    let mut line_start = 0;
    while line_start < content.len() {
        let line_end = match content[line_start..].find('\n') {
            Some(rel) => line_start + rel,
            None => content.len(),
        };
        let line = &content[line_start..line_end];
        let trimmed = line.trim();
        let mut policy = DiagnosticDirectivePolicy::default();
        let mut has_policy = false;
        let mut unused_diagnostic_index: i64 = -1;
        if trimmed == IGNORE_PREFIX {
            policy = DiagnosticDirectivePolicy::Ignore;
            has_policy = true;
        } else if let Some(message_text) = trimmed.strip_prefix(EXPECT_PREFIX) {
            policy = DiagnosticDirectivePolicy::Expect;
            has_policy = true;
            unused_diagnostic_index = unused_diagnostics.len() as i64;
            unused_diagnostics.push(UnusedExpectDirectiveDiagnostic { code: 2578, message_text: message_text.trim().to_string() });
        }
        if has_policy && line_end < content.len() {
            let affected_start = line_end + 1;
            let affected_length = match content[affected_start..].find('\n') {
                Some(length) => length,
                None => content.len() - affected_start,
            };
            let virtual_spans = mappings.original_to_virtual_spans(
                TextRange::new(affected_start as i32, (affected_start + affected_length) as i32),
                Feature::All,
            );
            if virtual_spans.len() == 1 {
                let mut directive = MappedDiagnosticDirective {
                    original_start: line_start as i64,
                    original_length: (line_end - line_start) as i64,
                    virtual_start: i64::from(virtual_spans[0].span.pos()),
                    virtual_end: i64::from(virtual_spans[0].span.end()),
                    policy,
                    ..Default::default()
                };
                if unused_diagnostic_index >= 0 {
                    directive.unused_expect_directive_index = Some(unused_diagnostic_index);
                }
                result.push(directive);
            }
        }
        if line_end == content.len() {
            break;
        }
        line_start = line_end + 1;
    }
    if unused_diagnostics.len() == 1 {
        for directive in &mut result {
            directive.unused_expect_directive_index = None;
        }
    }
    if result.is_empty() && unused_diagnostics.is_empty() {
        return None;
    }
    wrap(result, unused_diagnostics)
}

// transforming.go:267 (A member's raw json.Value is never empty; its text is the value's compact JSON.)
fn render_option(options: Option<&OrderedMap<String, Value>>, name: &str) -> String {
    if let Some(value) = options.and_then(|options| options.get(name)) {
        return json::marshal(value).unwrap_or_default();
    }
    "undefined".to_string()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
