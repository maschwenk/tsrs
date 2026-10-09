use std::io::{self, Write};
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicI32};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError};
use std::thread;
use std::time::Duration;

use tsrs_ast::MappedDiagnosticDirectivePolicy;
use tsrs_core::json::{self, Value};
use tsrs_core::{tspath, CompilerOptions, JsxEmit, Locale, ScriptTarget, Tristate, P};
use tsrs_ipc::{self as ipc, Closer, Conn, Handler, Protocol, ReadWriteCloser};
use tsrs_spanmap::{Fidelity, Kind, Segment};
use tsrs_tsoptions::gojson::compiler_options_to_go_json;
use tsrs_tsoptions::{Definition, Manifest, Mapper};

use crate::{
    new_host, new_host_with_options, unmarshal, DiagnosticDirectiveErrorKind, DiagnosticDirectivePolicy, DiagnosticDirectives,
    Error, HostOptions, InitializeErrorKind, InitializeParams, InitializeResult, MappedDiagnosticDirective, MappedOutput,
    OpenProjectParams, OpenProjectResult, OptionDiagnosticResult, PositionEncoding, ProjectErrorKind, ProjectSpec, ProtocolJson,
    Request, Spawner, SpawnerFunc, SupplementalOutput, TransformParams, TransformResult, TransformResultFiles,
    UnusedExpectDirectiveDiagnostic, CloseProjectParams, Diagnostic, METHOD_CLOSE_PROJECT, METHOD_INITIALIZE, METHOD_OPEN_PROJECT,
    METHOD_TRANSFORM,
};

// The tests build their mappers as Go does, with `&contentmapper.Mapper{...}`; the host keys them by address and holds
// `&'static Mapper`s (the command line's arena list in the compiler), so a test mapper is leaked.
fn leak(mapper: Mapper) -> &'static Mapper {
    Box::leak(Box::new(mapper))
}

// Go `&contentmapper.Mapper{Package: package, Extensions: extensions, Name: name, Version: version, Exec: exec}`.
fn new_mapper(package: &str, extensions: &[&str], name: &str, version: &str, exec: &[&str]) -> Mapper {
    Mapper {
        definition: Definition {
            package: package.to_string(),
            extensions: extensions.iter().map(|extension| extension.to_string()).collect(),
            ..Default::default()
        },
        manifest: Manifest {
            name: name.to_string(),
            version: version.to_string(),
            exec: exec.iter().map(|arg| arg.to_string()).collect(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn request<'a>(file_name: &'a str, content: &'a str) -> Request<'a> {
    Request { file_name, content }
}

fn compiler_options(options: CompilerOptions) -> Option<P<CompilerOptions>> {
    Some(P::new(options))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// Go's `json.Unmarshal(params, &p)` in a test mapper.
fn params<T: ProtocolJson>(params: Option<&Value>) -> Result<T, ipc::Error> {
    unmarshal(params.cloned()).map_err(ipc::Error::new)
}

// Go `fmt.Errorf("unexpected method %s", method)`.
fn unexpected_method(method: &str) -> ipc::Error {
    ipc::Error::new(format!("unexpected method {method}"))
}

// Go `spanmap.New(segments).Marshal()` as the json.Value a transform result carries.
fn marshal_span_map(segments: &[Segment]) -> Value {
    let data = tsrs_spanmap::new(segments).marshal().unwrap();
    json::unmarshal(std::str::from_utf8(&data).unwrap()).unwrap()
}

// A mapper of these tests. fakeSpawner wraps one that does not handle openProject and closeProject itself in
// noOpProjectMapper; Go asks with a type assertion for the `handlesProjects` marker method.
trait testMapper: Handler {
    fn handles_projects(&self) -> bool {
        false
    }
}

// fakeMapper is an in-process mapper that transforms content verbatim and reports one diagnostic.
// host_test.go:28
struct fakeMapper;

// host_test.go:30
struct responseMapper {
    response: Box<dyn Fn(TransformParams) -> Value + Send + Sync>,
}

fn response_mapper(response: impl Fn(TransformParams) -> Value + Send + Sync + 'static) -> responseMapper {
    responseMapper { response: Box::new(response) }
}

impl testMapper for responseMapper {}

impl Handler for responseMapper {
    // host_test.go:34
    fn handle_request(&self, method: &str, params_: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(InitializeResult { position_encoding: PositionEncoding::UTF8, diagnostic_source: "mapper".to_string() }
                .marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = params(params_)?;
                Ok((self.response)(p))
            }
            _ => Err(unexpected_method(method)),
        }
    }

    // host_test.go:49
    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        Ok(())
    }
}

impl testMapper for fakeMapper {}

impl Handler for fakeMapper {
    // host_test.go:53
    fn handle_request(&self, method: &str, params_: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => {
                Ok(InitializeResult { position_encoding: PositionEncoding::UTF8, diagnostic_source: "vue".to_string() }.marshal_json())
            }
            METHOD_TRANSFORM => {
                let p: TransformParams = params(params_)?;
                let mappings = marshal_span_map(&[Segment {
                    virtual_end: p.content.len() as i32,
                    original_end: p.content.len() as i32,
                    kind: Kind::Verbatim,
                    ..Default::default()
                }]);
                Ok(TransformResult {
                    mapped_output: MappedOutput {
                        text: p.content.clone(),
                        extension: ".ts".to_string(),
                        mappings: Some(mappings),
                        ..Default::default()
                    },
                    diagnostics: vec![Diagnostic {
                        message_text: "boom".to_string(),
                        start: 0,
                        length: 3.min(p.content.len()) as i64,
                        code: 9999,
                    }],
                    ..Default::default()
                }
                .marshal_json())
            }
            _ => Err(unexpected_method(method)),
        }
    }

    // host_test.go:186
    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        Ok(())
    }
}

// host_test.go:84
struct unicodeMapper {
    encoding: PositionEncoding,
    source: Option<String>,
}

// host_test.go:89
fn protocol_diagnostic_directives(
    directives: Vec<MappedDiagnosticDirective>,
    unused: Vec<UnusedExpectDirectiveDiagnostic>,
) -> Option<DiagnosticDirectives> {
    Some(DiagnosticDirectives { unused_expect_directive_diagnostics: unused, directives })
}

impl testMapper for unicodeMapper {}

impl Handler for unicodeMapper {
    // host_test.go:96
    fn handle_request(&self, method: &str, params_: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => {
                let p: InitializeParams = params(params_)?;
                let offered = p.position_encodings.contains(&self.encoding);
                if !offered && (self.encoding == PositionEncoding::UTF8 || self.encoding == PositionEncoding::UTF16) {
                    return Err(ipc::Error::new(format!("position encoding {:?} was not offered", self.encoding.0)));
                }
                let source = self.source.clone().unwrap_or_else(|| "mapper".to_string());
                Ok(InitializeResult { position_encoding: self.encoding.clone(), diagnostic_source: source }.marshal_json())
            }
            METHOD_TRANSFORM => {
                let p: TransformParams = params(params_)?;
                let (emoji_length, text_length): (i64, i64) = if self.encoding == PositionEncoding::UTF8 {
                    (2, 3)
                } else if self.encoding == PositionEncoding::UTF16 {
                    (1, 2)
                } else {
                    return Ok(TransformResult {
                        mapped_output: MappedOutput { text: p.content, extension: ".ts".to_string(), ..Default::default() },
                        ..Default::default()
                    }
                    .marshal_json());
                };
                let tuple = |values: [i64; 5]| Value::Array(values.iter().map(|&v| Value::Number(v as f64)).collect());
                let mappings = Value::Array(vec![
                    tuple([0, emoji_length, 0, emoji_length, i64::from(Kind::Verbatim.0)]),
                    tuple([
                        emoji_length,
                        text_length - emoji_length,
                        emoji_length,
                        text_length - emoji_length,
                        i64::from(Kind::Verbatim.0),
                    ]),
                ]);
                Ok(TransformResult {
                    mapped_output: MappedOutput {
                        text: p.content,
                        extension: ".ts".to_string(),
                        mappings: Some(mappings),
                        diagnostic_directives: protocol_diagnostic_directives(
                            vec![MappedDiagnosticDirective {
                                original_start: emoji_length,
                                original_length: text_length - emoji_length,
                                virtual_start: emoji_length,
                                virtual_end: text_length,
                                policy: DiagnosticDirectivePolicy::Ignore,
                                ..Default::default()
                            }],
                            Vec::new(),
                        ),
                    },
                    diagnostics: vec![Diagnostic {
                        message_text: "after non-ASCII character".to_string(),
                        start: emoji_length,
                        length: text_length - emoji_length,
                        code: 1001,
                    }],
                    ..Default::default()
                }
                .marshal_json())
            }
            _ => Err(unexpected_method(method)),
        }
    }

    // host_test.go:156
    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        Ok(())
    }
}

// host_test.go:160
struct invalidDiagnosticMapper {
    encoding: PositionEncoding,
}

impl testMapper for invalidDiagnosticMapper {}

impl Handler for invalidDiagnosticMapper {
    // host_test.go:164
    fn handle_request(&self, method: &str, _params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(InitializeResult { position_encoding: self.encoding.clone(), diagnostic_source: "mapper".to_string() }
                .marshal_json()),
            METHOD_TRANSFORM => Ok(TransformResult {
                mapped_output: MappedOutput { extension: ".ts".to_string(), ..Default::default() },
                diagnostics: vec![Diagnostic { message_text: "invalid boundary".to_string(), start: 1, code: 1002, ..Default::default() }],
                ..Default::default()
            }
            .marshal_json()),
            _ => Err(unexpected_method(method)),
        }
    }

    // host_test.go:182
    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        Ok(())
    }
}

// fakeSpawner serves each spawn request with an in-process mapper over a net.Pipe, counting spawns so
// tests can assert process consolidation. When handler is nil it serves a fakeMapper.
// host_test.go:190
#[derive(Default)]
struct fakeSpawner {
    spawns: AtomicI32,
    closes: Arc<AtomicI32>,
    handler: Option<Arc<dyn testMapper>>,
}

fn fake_spawner(handler: Option<Arc<dyn testMapper>>) -> Arc<fakeSpawner> {
    Arc::new(fakeSpawner { handler, ..Default::default() })
}

// host_test.go:196 (Go embeds the handler it wraps.)
struct noOpProjectMapper(Arc<dyn testMapper>);

impl Handler for noOpProjectMapper {
    // host_test.go:200
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_OPEN_PROJECT => Ok(OpenProjectResult::default().marshal_json()),
            METHOD_CLOSE_PROJECT => Ok(Value::Null),
            _ => self.0.handle_request(method, params),
        }
    }

    fn handle_notification(&self, method: &str, params: Option<&Value>) -> Result<(), ipc::Error> {
        self.0.handle_notification(method, params)
    }
}

impl Spawner for fakeSpawner {
    // host_test.go:211
    fn spawn(&self, _command: &[String], _dir: &str, _stderr: Option<Box<dyn Write + Send>>) -> Result<ReadWriteCloser, String> {
        self.spawns.fetch_add(1, SeqCst);
        let handler: Arc<dyn testMapper> = match &self.handler {
            Some(handler) => Arc::clone(handler),
            None => Arc::new(fakeMapper),
        };
        let handler: Arc<dyn Handler> = if handler.handles_projects() { handler } else { Arc::new(noOpProjectMapper(handler)) };
        let (client, server) = ipc::pipe().map_err(|err| err.to_string())?;
        thread::spawn(move || {
            let _ = ipc::new_async_conn(server, handler).run();
        });
        let closer = Arc::new(countingReadWriteCloser { closer: client.closer, closes: Arc::clone(&self.closes), once: Once::new() });
        Ok(ReadWriteCloser { reader: client.reader, writer: client.writer, closer })
    }
}

// host_test.go:225
struct countingReadWriteCloser {
    closer: Arc<dyn Closer>,
    closes: Arc<AtomicI32>,
    once: Once,
}

impl Closer for countingReadWriteCloser {
    // host_test.go:231
    fn close(&self) -> io::Result<()> {
        self.once.call_once(|| {
            self.closes.fetch_add(1, SeqCst);
        });
        self.closer.close()
    }
}

// host_test.go:236
#[test]
fn test_runner_transform() {
    let r = new_host(fake_spawner(None), Locale::DEFAULT);

    let mapper = leak(new_mapper("", &[".vue"], "vue", "1.0.0", &["vue-mapper"]));
    let result = r.transform(mapper, request("/a.vue", "export const x = 1;")).unwrap();
    assert_eq!(result.text, "export const x = 1;");
    assert_eq!(result.virtual_extension, ".ts");
    assert!(result.mappings.is_some());
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code(), 9999);
    assert_eq!(result.diagnostics[0].source(), "vue");
    r.close().unwrap();
}

// host_test.go:252
#[test]
fn test_host_logging() {
    let logs = Arc::new(Mutex::new(Vec::<String>::new()));
    let logger = {
        let logs = Arc::clone(&logs);
        Arc::new(move |message: &str| lock(&logs).push(message.to_string()))
    };
    let spawner = Arc::new(SpawnerFunc(|command: &[String], dir: &str, mut stderr: Option<Box<dyn Write + Send>>| {
        if let Some(stderr) = &mut stderr {
            let _ = stderr.write_all(b"mapper diagnostic\n");
        }
        fake_spawner(None).spawn(command, dir, stderr)
    }));
    let host = new_host_with_options(spawner, Locale::DEFAULT, HostOptions { logger: Some(logger) });
    let mapper = leak(new_mapper("configured", &[".vue"], "resolved", "1.0.0", &["mapper"]));
    host.transform(mapper, request("/a.vue", "export const x = 1;")).unwrap();

    let joined = lock(&logs).join("\n");
    assert!(joined.contains(r#"[content mapper: resolved] send: {"jsonrpc":"2.0","id":"api1","method":"initialize""#), "{joined}");
    assert!(joined.contains(r#"[content mapper: resolved] receive: {"jsonrpc":"2.0","id":"api1","result":"#), "{joined}");
    assert!(joined.contains("[content mapper: resolved] stderr: mapper diagnostic"), "{joined}");
    host.close().unwrap();
}

// host_test.go:282 (Go's io.Discard is None.)
#[test]
fn test_host_discards_stderr_without_logging() {
    let spawner = Arc::new(SpawnerFunc(|command: &[String], dir: &str, stderr: Option<Box<dyn Write + Send>>| {
        assert!(stderr.is_none());
        fake_spawner(None).spawn(command, dir, stderr)
    }));
    let host = new_host(spawner, Locale::DEFAULT);
    let mapper = leak(new_mapper("configured", &[".vue"], "resolved", "1.0.0", &["mapper"]));
    host.transform(mapper, request("/a.vue", "export const x = 1;")).unwrap();
    host.close().unwrap();
}

// host_test.go:298 TestMapperDiagnosticName is ported with Mapper, in tsrs_tsoptions/src/contentmappers_test.rs.

// host_test.go:313
#[test]
fn test_runner_transform_response_validation() {
    let mapper = leak(new_mapper("", &[".vue"], "mapper", "", &["mapper"]));

    // malformed result fails the request
    let host = new_host(
        fake_spawner(Some(Arc::new(response_mapper(|_| json::unmarshal(r#"{"text":1}"#).unwrap())))),
        Locale::DEFAULT,
    );
    assert!(host.transform(mapper, request("/a.vue", "a")).is_err());
    host.close().unwrap();
}

// host_test.go:425
struct closeSignalReadWriteCloser {
    closer: Arc<dyn Closer>,
    closed: Mutex<Sender<()>>,
    once: Once,
}

impl Closer for closeSignalReadWriteCloser {
    // host_test.go:430
    fn close(&self) -> io::Result<()> {
        let err = self.closer.close();
        self.once.call_once(|| {
            let _ = lock(&self.closed).send(());
        });
        err
    }
}

// The writer of a pipe end shared by a protocol and the test, which also writes to it directly (Go writes to the
// net.Conn it gave the protocol).
struct sharedWriter(Arc<Mutex<Box<dyn Write + Send>>>);

impl Write for sharedWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        lock(&self.0).write(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        lock(&self.0).flush()
    }
}

// host_test.go:329
#[test]
fn test_host_closes_process_when_read_loop_fails() {
    let (closed_tx, closed) = mpsc::channel();
    let closed_tx = Mutex::new(closed_tx);
    let spawner = Arc::new(SpawnerFunc(move |_: &[String], _: &str, _: Option<Box<dyn Write + Send>>| {
        let (client, server) = ipc::pipe().map_err(|err| err.to_string())?;
        thread::spawn(move || {
            let ReadWriteCloser { reader, writer, closer } = server;
            let writer = Arc::new(Mutex::new(writer));
            let protocol = ipc::new_jsonrpc_protocol(reader, sharedWriter(Arc::clone(&writer)));
            let message = protocol.read_message().unwrap();
            assert_eq!(message.method, METHOD_INITIALIZE);
            protocol
                .write_response(
                    message.id.as_ref().unwrap(),
                    &InitializeResult { position_encoding: PositionEncoding::UTF8, diagnostic_source: "mapper".to_string() }.marshal_json(),
                )
                .unwrap();
            lock(&writer).write_all(b"oops\n").unwrap();
            let _ = closer.close();
        });
        let closer = Arc::new(closeSignalReadWriteCloser {
            closer: client.closer,
            closed: Mutex::new(lock(&closed_tx).clone()),
            once: Once::new(),
        });
        Ok(ReadWriteCloser { reader: client.reader, writer: client.writer, closer })
    }));
    let host = new_host(spawner, Locale::DEFAULT);
    let mapper = leak(new_mapper("", &[], "mapper", "", &["mapper"]));
    assert!(host.transform(mapper, request("/a.vue", "")).is_err());
    let process_closed = closed.recv_timeout(Duration::from_secs(1)).is_ok();
    assert!(process_closed, "mapper process was not closed after its read loop failed");
    host.close().unwrap();
}

// host_test.go:410
#[derive(Default)]
struct exitOnCloseReadWriteCloser {
    closer: Option<Arc<dyn Closer>>,
    exited: AtomicBool,
}

impl Closer for exitOnCloseReadWriteCloser {
    // host_test.go:415
    fn close(&self) -> io::Result<()> {
        self.exited.store(true, SeqCst);
        self.closer.as_ref().map_or(Ok(()), |closer| closer.close())
    }

    // host_test.go:420
    fn exit_code(&self) -> Option<i32> {
        self.exited.load(SeqCst).then_some(1)
    }
}

// host_test.go:364 (The mapper waits for the end of the test, Go's `<-t.Context().Done()`, which here is the drop of
// `test_done`. The test takes the initialize timeout, five seconds.)
#[test]
fn test_host_reports_initialization_timeout_before_closing_process() {
    let (test_done, done) = mpsc::channel::<()>();
    let done = Arc::new(Mutex::new(done));
    let spawner = Arc::new(SpawnerFunc(move |_: &[String], _: &str, _: Option<Box<dyn Write + Send>>| {
        let (client, server) = ipc::pipe().map_err(|err| err.to_string())?;
        let done = Arc::clone(&done);
        thread::spawn(move || {
            let ReadWriteCloser { reader, writer, closer } = server;
            let _ = ipc::new_jsonrpc_protocol(reader, writer).read_message();
            let _ = lock(&done).recv();
            let _ = closer.close();
        });
        let closer = Arc::new(exitOnCloseReadWriteCloser { closer: Some(client.closer), ..Default::default() });
        Ok(ReadWriteCloser { reader: client.reader, writer: client.writer, closer })
    }));
    let host = new_host(spawner, Locale::DEFAULT);
    let mapper = leak(new_mapper("", &[], "mapper", "", &["mapper"]));
    let err = host.transform(mapper, request("/a.vue", "")).unwrap_err();
    let initialize_error = err.as_initialize_error().unwrap_or_else(|| panic!("expected InitializeError, got {err}"));
    assert_eq!(initialize_error.kind, InitializeErrorKind::NoResponse);
    host.close().unwrap();
    drop(test_done);
}

// host_test.go:401
struct exitedReadWriteCloser {
    closer: Arc<dyn Closer>,
    exit_code: i32,
}

impl Closer for exitedReadWriteCloser {
    fn close(&self) -> io::Result<()> {
        self.closer.close()
    }

    // host_test.go:406
    fn exit_code(&self) -> Option<i32> {
        Some(self.exit_code)
    }
}

// host_test.go:384
#[test]
fn test_host_reports_process_exit_before_initialization() {
    let spawner = Arc::new(SpawnerFunc(|_: &[String], _: &str, _: Option<Box<dyn Write + Send>>| {
        let (client, server) = ipc::pipe().map_err(|err| err.to_string())?;
        server.closer.close().unwrap();
        drop(server);
        let closer = Arc::new(exitedReadWriteCloser { closer: client.closer, exit_code: 42 });
        Ok(ReadWriteCloser { reader: client.reader, writer: client.writer, closer })
    }));
    let host = new_host(spawner, Locale::DEFAULT);
    let mapper = leak(new_mapper("", &[], "mapper", "", &["mapper"]));
    let err = host.transform(mapper, request("/a.vue", "")).unwrap_err();
    let initialize_error = err.as_initialize_error().unwrap_or_else(|| panic!("expected InitializeError, got {err}"));
    assert_eq!(initialize_error.kind, InitializeErrorKind::ProcessExit);
    assert_eq!(initialize_error.exit_code, 42);
    host.close().unwrap();
}

// host_test.go:436
#[test]
fn test_runner_transform_diagnostic_directives() {
    let mapper = leak(new_mapper("", &[".vue"], "mapper", "", &["mapper"]));
    let transform = |output: MappedOutput| -> Result<TransformResultFiles, Error> {
        let host = new_host(
            fake_spawner(Some(Arc::new(response_mapper(move |_| {
                TransformResult { mapped_output: output.clone(), ..Default::default() }.marshal_json()
            })))),
            Locale::DEFAULT,
        );
        let result = host.transform(mapper, request("/a.vue", "directive\nsource"));
        host.close().unwrap();
        result
    };

    let result = transform(MappedOutput {
        text: "virtual source".to_string(),
        extension: ".ts".to_string(),
        diagnostic_directives: protocol_diagnostic_directives(
            vec![MappedDiagnosticDirective {
                original_start: 0,
                original_length: 9,
                virtual_start: 8,
                virtual_end: 14,
                policy: DiagnosticDirectivePolicy::Expect,
                ..Default::default()
            }],
            vec![UnusedExpectDirectiveDiagnostic { code: 2578, message_text: "Unused framework directive.".to_string() }],
        ),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(result.diagnostic_directives.len(), 1);
    let directive = result.diagnostic_directives[0];
    assert_eq!(directive.original_range.pos(), 0);
    assert_eq!(directive.original_range.end(), 9);
    assert_eq!(directive.virtual_range.pos(), 8);
    assert_eq!(directive.virtual_range.end(), 14);
    assert_eq!(directive.policy, MappedDiagnosticDirectivePolicy::Expect);
    assert_eq!(directive.unused_code, 2578);
    assert_eq!(directive.unused_message_text, "Unused framework directive.");
    assert_eq!(directive.source, "mapper");
    let result = transform(MappedOutput {
        text: "virtual source".to_string(),
        extension: ".ts".to_string(),
        diagnostic_directives: protocol_diagnostic_directives(
            vec![MappedDiagnosticDirective {
                original_length: 9,
                virtual_start: 8,
                virtual_end: 14,
                policy: DiagnosticDirectivePolicy::Expect,
                unused_expect_directive_index: Some(1),
                ..Default::default()
            }],
            vec![
                UnusedExpectDirectiveDiagnostic { code: 1, message_text: "first".to_string() },
                UnusedExpectDirectiveDiagnostic { code: 2, message_text: "second".to_string() },
            ],
        ),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(result.diagnostic_directives[0].unused_code, 2);
    assert_eq!(result.diagnostic_directives[0].unused_message_text, "second");
    transform(MappedOutput {
        text: "x".to_string(),
        extension: ".ts".to_string(),
        diagnostic_directives: protocol_diagnostic_directives(
            vec![MappedDiagnosticDirective { original_start: -1, policy: DiagnosticDirectivePolicy::Ignore, ..Default::default() }],
            vec![UnusedExpectDirectiveDiagnostic::default()],
        ),
        ..Default::default()
    })
    .unwrap();

    let directive = |virtual_start: i64, virtual_end: i64, original_start: i64, policy: DiagnosticDirectivePolicy| {
        MappedDiagnosticDirective { original_start, virtual_start, virtual_end, policy, ..Default::default() }
    };
    let ignore = DiagnosticDirectivePolicy::Ignore;
    let expect = DiagnosticDirectivePolicy::Expect;
    let invalid = [
        ("invalid range", "x", vec![directive(-1, 0, 0, ignore)], DiagnosticDirectiveErrorKind::InvalidRange),
        ("unknown policy", "x", vec![directive(0, 0, 0, DiagnosticDirectivePolicy(2))], DiagnosticDirectiveErrorKind::InvalidPolicy),
        (
            "expect requires unused diagnostic",
            "x",
            vec![directive(0, 0, 0, expect)],
            DiagnosticDirectiveErrorKind::ExpectMissingUnusedDiagnostic,
        ),
        (
            "multiple unused diagnostics require index",
            "x",
            vec![directive(0, 0, 0, expect)],
            DiagnosticDirectiveErrorKind::ExpectMissingUnusedDiagnostic,
        ),
        ("original range out of bounds", "x", vec![directive(0, 0, 99, expect)], DiagnosticDirectiveErrorKind::InvalidRange),
        ("virtual range out of bounds", "x", vec![directive(99, 0, 0, ignore)], DiagnosticDirectiveErrorKind::InvalidRange),
        ("overlap", "abc", vec![directive(0, 2, 0, ignore), directive(1, 3, 0, ignore)], DiagnosticDirectiveErrorKind::Overlap),
    ];
    for (name, text, directives, kind) in invalid {
        let mut diagnostic_directives = protocol_diagnostic_directives(directives, Vec::new()).unwrap();
        if name == "original range out of bounds" {
            diagnostic_directives.unused_expect_directive_diagnostics = vec![UnusedExpectDirectiveDiagnostic::default()];
        } else if name == "multiple unused diagnostics require index" {
            diagnostic_directives.unused_expect_directive_diagnostics =
                vec![UnusedExpectDirectiveDiagnostic::default(), UnusedExpectDirectiveDiagnostic::default()];
        }
        let transform_err = transform(MappedOutput {
            text: text.to_string(),
            extension: ".ts".to_string(),
            diagnostic_directives: Some(diagnostic_directives),
            ..Default::default()
        })
        .unwrap_err();
        let directive_error = transform_err.as_diagnostic_directive_error().unwrap_or_else(|| panic!("{name}: {transform_err}"));
        assert_eq!(directive_error.kind, kind, "{name}");
    }
}

// host_test.go:579
#[test]
fn test_mapped_diagnostic_directive_json() {
    let directive = |policy: DiagnosticDirectivePolicy| MappedDiagnosticDirective {
        virtual_start: 8,
        virtual_end: 14,
        original_start: 0,
        original_length: 9,
        policy,
        ..Default::default()
    };
    let tests = [
        ("ignore", directive(DiagnosticDirectivePolicy::Ignore), "[0,9,8,14,0]"),
        ("expect", directive(DiagnosticDirectivePolicy::Expect), "[0,9,8,14,1]"),
    ];
    for (name, directive, want) in tests {
        let data = json::marshal(&directive.marshal_json()).unwrap();
        assert_eq!(data, want, "{name}");
        let decoded = MappedDiagnosticDirective::unmarshal_json(json::unmarshal(&data).unwrap(), "").unwrap();
        assert_eq!(decoded, directive, "{name}");
    }

    for data in ["[0,0,0,0]", "[0,0,0,0,0,1,2]"] {
        let err = MappedDiagnosticDirective::unmarshal_json(json::unmarshal(data).unwrap(), "").unwrap_err();
        assert!(err.contains("diagnostic directive tuple"), "{err}");
    }

    let diagnostic_directives = DiagnosticDirectives {
        unused_expect_directive_diagnostics: vec![
            UnusedExpectDirectiveDiagnostic { code: 1, message_text: "first".to_string() },
            UnusedExpectDirectiveDiagnostic { code: 2, message_text: "second".to_string() },
        ],
        directives: vec![MappedDiagnosticDirective {
            original_start: 2,
            original_length: 3,
            virtual_start: 5,
            virtual_end: 9,
            policy: DiagnosticDirectivePolicy::Expect,
            unused_expect_directive_index: Some(1),
        }],
    };
    let data = json::marshal(&diagnostic_directives.marshal_json()).unwrap();
    assert_eq!(
        data,
        r#"{"unusedExpectDirectiveDiagnostics":[{"code":1,"messageText":"first"},{"code":2,"messageText":"second"}],"directives":[[2,3,5,9,1,1]]}"#
    );
    let decoded = DiagnosticDirectives::unmarshal_json(json::unmarshal(&data).unwrap(), "").unwrap();
    assert_eq!(decoded, diagnostic_directives);
}

// host_test.go:646
#[test]
fn test_runner_transform_supplemental_outputs() {
    let host = new_host(
        fake_spawner(Some(Arc::new(response_mapper(|_| {
            TransformResult {
                mapped_output: MappedOutput { text: "export default 1;".to_string(), extension: ".ts".to_string(), ..Default::default() },
                supplemental: vec![
                    SupplementalOutput {
                        mapped_output: MappedOutput {
                            text: "declare const first: string;".to_string(),
                            extension: ".ts".to_string(),
                            diagnostic_directives: protocol_diagnostic_directives(
                                vec![MappedDiagnosticDirective {
                                    virtual_end: 7,
                                    policy: DiagnosticDirectivePolicy::Ignore,
                                    ..Default::default()
                                }],
                                Vec::new(),
                            ),
                            ..Default::default()
                        },
                    },
                    SupplementalOutput {
                        mapped_output: MappedOutput {
                            text: "declare const second: number;".to_string(),
                            extension: ".mjs".to_string(),
                            ..Default::default()
                        },
                    },
                ],
                ..Default::default()
            }
            .marshal_json()
        })))),
        Locale::DEFAULT,
    );
    let mapper = leak(new_mapper("", &[".vue"], "mapper", "", &["mapper"]));
    let result = host.transform(mapper, request("/component.vue", "component")).unwrap();
    assert_eq!(result.supplemental.len(), 2);
    assert_eq!(result.supplemental[0].text, "declare const first: string;");
    assert_eq!(result.supplemental[0].virtual_extension, ".ts");
    assert!(result.supplemental[0].mappings.is_some());
    assert_eq!(result.supplemental[0].diagnostic_directives.len(), 1);
    assert_eq!(result.supplemental[0].diagnostic_directives[0].virtual_range.end(), 7);
    assert_eq!(result.supplemental[1].virtual_extension, ".mjs");
    assert!(result.supplemental[1].mappings.is_some());
    host.close().unwrap();
}

// host_test.go:677
#[test]
fn test_runner_transform_invalid_supplemental_diagnostic_directive() {
    let host = new_host(
        fake_spawner(Some(Arc::new(response_mapper(|_| {
            let output = |diagnostic_directives| MappedOutput {
                text: "export {};".to_string(),
                extension: ".ts".to_string(),
                diagnostic_directives,
                ..Default::default()
            };
            TransformResult {
                mapped_output: output(None),
                supplemental: vec![
                    SupplementalOutput { mapped_output: output(None) },
                    SupplementalOutput {
                        mapped_output: output(protocol_diagnostic_directives(
                            vec![MappedDiagnosticDirective { policy: DiagnosticDirectivePolicy::Expect, ..Default::default() }],
                            Vec::new(),
                        )),
                    },
                ],
                ..Default::default()
            }
            .marshal_json()
        })))),
        Locale::DEFAULT,
    );
    let mapper = leak(new_mapper("", &[".vue"], "mapper", "", &["mapper"]));
    let err = host.transform(mapper, request("/component.vue", "component")).unwrap_err();
    let directive_error = err.as_diagnostic_directive_error().unwrap_or_else(|| panic!("{err}"));
    assert_eq!(directive_error.kind, DiagnosticDirectiveErrorKind::ExpectMissingUnusedDiagnostic);
    assert_eq!(directive_error.index, 0);
    assert_eq!(directive_error.supplemental_index, 1);
    host.close().unwrap();
}

// host_test.go:703
#[test]
fn test_runner_rejects_invalid_virtual_extension() {
    for supplemental in [false, true] {
        for extension in ["", ".coffee"] {
            let host = new_host(
                fake_spawner(Some(Arc::new(response_mapper(move |_| {
                    let mut canonical_extension = extension;
                    let mut supplemental_outputs = Vec::new();
                    if supplemental {
                        canonical_extension = ".ts";
                        supplemental_outputs = vec![SupplementalOutput {
                            mapped_output: MappedOutput {
                                text: "export {};".to_string(),
                                extension: extension.to_string(),
                                ..Default::default()
                            },
                        }];
                    }
                    TransformResult {
                        mapped_output: MappedOutput {
                            text: "export {};".to_string(),
                            extension: canonical_extension.to_string(),
                            ..Default::default()
                        },
                        supplemental: supplemental_outputs,
                        ..Default::default()
                    }
                    .marshal_json()
                })))),
                Locale::DEFAULT,
            );
            let mapper = leak(new_mapper("", &[".vue"], "mapper", "", &["mapper"]));
            let err = host.transform(mapper, request("/component.vue", "component")).unwrap_err();
            assert!(err.to_string().contains("invalid virtual extension"), "supplemental={supplemental}/{extension}: {err}");
            host.close().unwrap();
        }
    }
}

// host_test.go:730
#[test]
fn test_runner_position_encodings() {
    for encoding in [PositionEncoding::UTF8, PositionEncoding::UTF16] {
        let r = new_host(fake_spawner(Some(Arc::new(unicodeMapper { encoding: encoding.clone(), source: None }))), Locale::DEFAULT);
        let mapper = leak(new_mapper("", &[], &encoding.0, "", &["mapper"]));
        let result = r.transform(mapper, request("/a.vue", "éx")).unwrap();
        let mappings = result.mappings.unwrap();
        let segments = mappings.segments();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].virtual_end, 2);
        assert_eq!(segments[0].original_end, 2);
        assert_eq!(segments[1].virtual_start, 2);
        assert_eq!(segments[1].original_start, 2);
        assert_eq!(result.text, "éx");
        let problem = mappings.validate(&result.text, "éx");
        assert!(problem.is_none(), "{problem:?}");
        let (mapped, fidelity) = mappings.virtual_to_original_position(2);
        assert_eq!(mapped, 2);
        assert_eq!(fidelity, Fidelity::Exact);
        assert_eq!(result.diagnostics[0].pos(), 2);
        assert_eq!(result.diagnostics[0].end(), 3);
        assert_eq!(result.diagnostic_directives[0].original_range.pos(), 2);
        assert_eq!(result.diagnostic_directives[0].original_range.end(), 3);
        assert_eq!(result.diagnostic_directives[0].virtual_range.pos(), 2);
        assert_eq!(result.diagnostic_directives[0].virtual_range.end(), 3);
        r.close().unwrap();
    }
}

// host_test.go:765
#[test]
fn test_runner_rejects_unsupported_position_encoding() {
    let encoding = PositionEncoding(std::borrow::Cow::Borrowed("utf-32"));
    let r = new_host(fake_spawner(Some(Arc::new(unicodeMapper { encoding, source: None }))), Locale::DEFAULT);
    let mapper = leak(new_mapper("", &[], "invalid", "", &["mapper"]));
    let err = r.transform(mapper, request("/a.vue", "x")).unwrap_err();
    assert!(err.to_string().contains("unsupported position encoding"), "{err}");
    r.close().unwrap();
}

// host_test.go:774
#[test]
fn test_runner_rejects_invalid_diagnostic_source() {
    for source in ["", " ", "ts", "TS", "d.ts", "json", "typescript", "TypeScript", "tsc", "TSC"] {
        let handler = unicodeMapper { encoding: PositionEncoding::UTF8, source: Some(source.to_string()) };
        let r = new_host(fake_spawner(Some(Arc::new(handler))), Locale::DEFAULT);
        let mapper = leak(new_mapper("", &[], "invalid", "", &["mapper"]));
        let err = r.transform(mapper, request("/a.vue", "x")).unwrap_err().to_string();
        if source.trim().is_empty() {
            assert!(err.contains("diagnostic source must not be empty"), "{source:?}: {err}");
        } else {
            assert!(err.contains("is reserved by TypeScript"), "{source:?}: {err}");
        }
        r.close().unwrap();
    }
}

// host_test.go:793
#[test]
fn test_runner_rejects_positions_inside_unicode_characters() {
    for (encoding, content) in [(PositionEncoding::UTF8, "é"), (PositionEncoding::UTF16, "😀")] {
        let r = new_host(fake_spawner(Some(Arc::new(invalidDiagnosticMapper { encoding: encoding.clone() }))), Locale::DEFAULT);
        let mapper = leak(new_mapper("", &[], &encoding.0, "", &["mapper"]));
        let err = r.transform(mapper, request("/a.vue", content)).unwrap_err();
        assert!(err.to_string().contains("splits a Unicode code point"), "{encoding}: {err}");
        r.close().unwrap();
    }
}

// host_test.go:813
#[test]
fn test_runner_consolidates_by_identity() {
    let spawner = fake_spawner(None);
    let r = new_host(Arc::clone(&spawner) as Arc<dyn Spawner>, Locale::DEFAULT);

    // Two logically-separate mappers with the same identity share one process.
    let vue_a = leak(new_mapper("a", &[], "vue", "1.0.0", &["vue-mapper"]));
    let vue_b = leak(new_mapper("b", &[], "vue", "1.0.0", &["vue-mapper"]));
    let svelte = leak(new_mapper("", &[], "svelte", "2.0.0", &["svelte-mapper"]));
    let project = r
        .project(ProjectSpec {
            mappers: vec![vue_a, vue_b, svelte],
            compiler_options: compiler_options(CompilerOptions::default()),
            ..Default::default()
        })
        .unwrap();

    for m in [vue_a, vue_b, vue_a, svelte] {
        project.transform(m, request("/x", "y")).unwrap();
    }
    assert_eq!(spawner.spawns.load(SeqCst), 2, "expected one process per identity");
    project.close().unwrap();
    r.close().unwrap();
}

// host_test.go:833
#[test]
fn test_runner_lease_lifecycle() {
    let spawner = fake_spawner(None);
    let r = new_host(Arc::clone(&spawner) as Arc<dyn Spawner>, Locale::DEFAULT);

    let vue_a = leak(new_mapper("a", &[], "vue", "1.0.0", &["vue-mapper"]));
    let vue_b = leak(new_mapper("b", &[], "vue", "1.0.0", &["vue-mapper"]));
    let svelte = leak(new_mapper("", &[], "svelte", "2.0.0", &["svelte-mapper"]));

    let release_vue_a = r.acquire(&[vue_a, vue_a]);
    let release_vue_b = r.acquire(&[vue_b]);
    let release_svelte = r.acquire(&[svelte]);
    for mapper in [vue_a, svelte] {
        r.transform(mapper, request("/x", "y")).unwrap();
    }
    assert_eq!(spawner.spawns.load(SeqCst), 2);

    release_vue_a();
    assert_eq!(spawner.closes.load(SeqCst), 0, "shared vue process should remain owned");
    release_svelte();
    assert_eq!(spawner.closes.load(SeqCst), 1, "final release should close the process");
    release_vue_b();
    release_vue_b();
    assert_eq!(spawner.closes.load(SeqCst), 2, "final vue owner should close once");

    let release_new = r.acquire(&[vue_a]);
    r.transform(vue_a, request("/x", "y")).unwrap();
    assert_eq!(spawner.spawns.load(SeqCst), 3, "reacquiring should spawn a fresh process lazily");
    release_new();
    assert_eq!(spawner.closes.load(SeqCst), 3);
    r.close().unwrap();
}

// recordingMapper captures project configuration and lifecycle requests for host protocol tests.
// host_test.go:868 (The fields Go guards with mu; the configuration fields are set before the mapper is used.)
#[derive(Default)]
struct recordingMapper {
    mu: Mutex<recordingMapperState>,
    watched_files: Option<Vec<String>>,
    config_identity: Option<String>,
    dynamic_config: bool,
    option_diagnostics: Vec<OptionDiagnosticResult>,
}

#[derive(Default)]
struct recordingMapperState {
    received: String,
    received_options: String,
    received_locale: String,
    project_handles: Vec<String>,
    closed_handles: Vec<String>,
    transform_handle: String,
    transform_params: String,
}

// host_test.go:884
struct blockingMapper {
    recording_mapper: recordingMapper,
    // Go's `close(started)`: dropping the sender.
    started: Mutex<Option<Sender<()>>>,
    // Go's `<-proceed`, which the test's `close(proceed)` releases: the receiver sees the sender dropped.
    proceed: Mutex<Receiver<()>>,
}

// host_test.go:890 handlesProjects
impl testMapper for recordingMapper {
    fn handles_projects(&self) -> bool {
        true
    }
}

impl testMapper for blockingMapper {
    fn handles_projects(&self) -> bool {
        true
    }
}

impl Handler for blockingMapper {
    // host_test.go:892
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        if method == METHOD_TRANSFORM {
            drop(lock(&self.started).take());
            let _ = lock(&self.proceed).recv();
        }
        self.recording_mapper.handle_request(method, params)
    }

    fn handle_notification(&self, method: &str, params: Option<&Value>) -> Result<(), ipc::Error> {
        self.recording_mapper.handle_notification(method, params)
    }
}

// Go `string(raw)` of a json.Value: "" when it is empty (absent), else its JSON text.
fn raw_text(raw: Option<&Value>) -> String {
    raw.map(|value| json::marshal(value).unwrap()).unwrap_or_default()
}

impl Handler for recordingMapper {
    // host_test.go:900
    fn handle_request(&self, method: &str, params_: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => {
                let p: InitializeParams = params(params_)?;
                lock(&self.mu).received_locale = p.locale;
                Ok(InitializeResult { position_encoding: PositionEncoding::UTF8, diagnostic_source: "mapper".to_string() }.marshal_json())
            }
            METHOD_OPEN_PROJECT => {
                let p: OpenProjectParams = params(params_)?;
                let raw = raw_text(p.compiler_options.as_ref());
                let mut watched_files = self.watched_files.clone();
                {
                    let mut state = lock(&self.mu);
                    state.project_handles.push(p.project_handle.clone());
                    state.received = raw;
                    state.received_options = raw_text(p.options.as_ref());
                }
                let dynamic_config = self.dynamic_config;
                let config_identity_override = self.config_identity.clone();
                let option_diagnostics = self.option_diagnostics.clone();
                if !dynamic_config && watched_files.is_none() && config_identity_override.is_none() && option_diagnostics.is_empty() {
                    return Ok(OpenProjectResult::default().marshal_json());
                }
                if dynamic_config && watched_files.is_none() {
                    watched_files =
                        Some(vec![tspath::combine_paths(&tspath::get_directory_path(&p.config_file_name), &["mapper.config.js"])]);
                }
                let mut config_identity = String::new();
                if dynamic_config {
                    config_identity = format!("config:{}", raw_text(p.options.as_ref()));
                }
                if let Some(config_identity_override) = config_identity_override {
                    config_identity = config_identity_override;
                }
                Ok(OpenProjectResult { config_identity, watched_files: watched_files.unwrap_or_default(), option_diagnostics }.marshal_json())
            }
            METHOD_CLOSE_PROJECT => {
                let p: CloseProjectParams = params(params_)?;
                lock(&self.mu).closed_handles.push(p.project_handle);
                Ok(Value::Null)
            }
            METHOD_TRANSFORM => {
                let p: TransformParams = params(params_)?;
                {
                    let mut state = lock(&self.mu);
                    state.transform_handle = p.project_handle.clone();
                    state.transform_params = raw_text(params_);
                }
                Ok(TransformResult {
                    mapped_output: MappedOutput { text: p.content, extension: ".ts".to_string(), ..Default::default() },
                    ..Default::default()
                }
                .marshal_json())
            }
            _ => Err(unexpected_method(method)),
        }
    }

    // host_test.go:1211
    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        Ok(())
    }
}

// Go `&contentmapper.Mapper{Options: options, Name: name, Version: "1.0.0", Exec: []string{"mapper"}, CompilerOptions:
// compiler_options, DynamicConfig: dynamic_config}`.
fn project_mapper(options: &str, name: &str, compiler_options: &[&str], dynamic_config: bool) -> &'static Mapper {
    let mut mapper = new_mapper("", &[], name, "1.0.0", &["mapper"]);
    mapper.definition.options = options.to_string();
    mapper.manifest.compiler_options = compiler_options.iter().map(|option| option.to_string()).collect();
    mapper.manifest.dynamic_config = dynamic_config;
    leak(mapper)
}

// host_test.go:971
#[test]
fn test_project_lifecycle() {
    let mapper_process = Arc::new(recordingMapper { dynamic_config: true, ..Default::default() });
    let spawner = fake_spawner(Some(Arc::clone(&mapper_process) as Arc<dyn testMapper>));
    let host = new_host(Arc::clone(&spawner) as Arc<dyn Spawner>, Locale::DEFAULT);

    let static_mapper = project_mapper(r#"{"mode":"static"}"#, "static", &[], false);
    let static_project = host
        .project(ProjectSpec {
            config_file_name: "/repo/tsconfig.json".to_string(),
            mappers: vec![static_mapper],
            compiler_options: compiler_options(CompilerOptions::default()),
        })
        .unwrap();
    assert_eq!(spawner.spawns.load(SeqCst), 0, "static identity should not spawn the mapper");
    let static_identities = static_project.identities().unwrap();
    assert_eq!(static_identities.len(), 1);
    static_project.close().unwrap();

    let dynamic_a = project_mapper(r#"{"mode":"a"}"#, "dynamic", &["jsx"], true);
    let dynamic_b = project_mapper(r#"{"mode":"b"}"#, "dynamic", &[], true);
    let dynamic_a_options = compiler_options(CompilerOptions::default());
    let project = |config_file_name: &str, mappers: Vec<&'static Mapper>, compiler_options: Option<P<CompilerOptions>>| {
        host.project(ProjectSpec { config_file_name: config_file_name.to_string(), mappers, compiler_options }).unwrap()
    };
    let project_a = project("/repo/a/tsconfig.json", vec![dynamic_a, dynamic_b], dynamic_a_options);
    let project_a_reversed = project("/repo/reversed/tsconfig.json", vec![dynamic_b, dynamic_a], dynamic_a_options);
    let project_different_options = project(
        "/repo/options/tsconfig.json",
        vec![dynamic_a],
        compiler_options(CompilerOptions { jsx: JsxEmit::React, ..Default::default() }),
    );
    let project_b = project("/repo/b/tsconfig.json", vec![dynamic_a], compiler_options(CompilerOptions::default()));
    let project_a_again = project("/repo/a/tsconfig.json", vec![dynamic_a, dynamic_b], dynamic_a_options);
    assert_eq!(spawner.spawns.load(SeqCst), 0, "getting dynamic projects should not start the mapper");
    let project_a_identities = project_a.identities().unwrap();
    let project_a_reversed_identities = project_a_reversed.identities().unwrap();
    let project_different_option_identities = project_different_options.identities().unwrap();
    let project_b_identities = project_b.identities().unwrap();
    assert_eq!(project_a_identities.len(), 2);
    assert_eq!(project_a_reversed_identities.len(), 2);
    assert_eq!(project_a_identities[0], project_a_reversed_identities[1]);
    assert_eq!(project_a_identities[1], project_a_reversed_identities[0]);
    assert_ne!(project_a_identities[0], project_different_option_identities[0]);
    assert_eq!(project_b_identities.len(), 1);
    assert_eq!(spawner.spawns.load(SeqCst), 1, "dynamic projects should share one mapper process");
    let project_a_watched_files = project_a.watched_files().unwrap();
    let project_b_watched_files = project_b.watched_files().unwrap();
    assert_eq!(project_a_watched_files.len(), 1);
    assert_eq!(project_b_watched_files.len(), 1);

    project_a.transform(dynamic_b, request("/repo/a/file.ext", "x")).unwrap();
    {
        let state = lock(&mapper_process.mu);
        assert!(state.project_handles[..2].contains(&state.transform_handle));
    }

    project_a_again.close().unwrap();
    project_a.close().unwrap();
    project_a_reversed.close().unwrap();
    project_different_options.close().unwrap();
    project_b.close().unwrap();
    let timings = host.timings();
    let dynamic_timings = timings.mappers[&dynamic_a.identity()];
    assert_eq!(dynamic_timings.spawn.count, 1);
    assert_eq!(dynamic_timings.initialize.count, 1);
    assert_eq!(dynamic_timings.open_project.count, 6);
    assert_eq!(dynamic_timings.transform.count, 1);
    assert_eq!(dynamic_timings.close_project.count, 6);
    let state = lock(&mapper_process.mu);
    assert_eq!(state.project_handles.len(), 6);
    assert_eq!(state.closed_handles.len(), 6);
    drop(state);
    host.close().unwrap();
}

// host_test.go:1074
#[test]
fn test_project_methods_after_host_close() {
    let mapper_process = Arc::new(recordingMapper { dynamic_config: true, ..Default::default() });
    let host = new_host(fake_spawner(Some(mapper_process)), Locale::DEFAULT);
    let mapper = project_mapper("", "dynamic", &[], true);
    let project = host
        .project(ProjectSpec {
            config_file_name: "/repo/tsconfig.json".to_string(),
            mappers: vec![mapper],
            compiler_options: compiler_options(CompilerOptions::default()),
        })
        .unwrap();
    project.transform(mapper, request("/repo/file.ext", "x")).unwrap();
    let identity = project.identity(&Mapper::default()).unwrap();
    assert_eq!(identity, "");
    host.close().unwrap();

    project.refresh().unwrap();
    let identities = project.identities().unwrap();
    assert_eq!(identities.len(), 0);
    let identity = project.identity(mapper).unwrap();
    assert_eq!(identity, "");
    let identity = project.identity(&Mapper::default()).unwrap();
    assert_eq!(identity, "");
    let watched_files = project.watched_files().unwrap();
    assert_eq!(watched_files.len(), 0);
    assert_eq!(project.diagnostics().len(), 0);
    let err = project.transform(mapper, request("/repo/file.ext", "x")).unwrap_err();
    assert!(err.to_string().contains("content mapper project is closed"), "{err}");
    let _ = project.close();
}

// Runs one transform of a project whose mapper is `mapper_process` and returns its error's ProjectError kind (Go's
// errors.AsType of a *TransformError, then of a *ProjectError in it).
fn project_error_kind(mapper_process: recordingMapper, project_mapper_: &'static Mapper) -> ProjectErrorKind {
    let host = new_host(fake_spawner(Some(Arc::new(mapper_process))), Locale::DEFAULT);
    let project = host
        .project(ProjectSpec {
            config_file_name: "/repo/tsconfig.json".to_string(),
            mappers: vec![project_mapper_],
            compiler_options: compiler_options(CompilerOptions::default()),
        })
        .unwrap();
    let err = project.transform(project_mapper_, request("/repo/file.ext", "x")).unwrap_err();
    let transform_error = err.as_transform_error().unwrap_or_else(|| panic!("expected TransformError, got {err}"));
    let project_error =
        Error::Transform(transform_error.clone()).as_project_error().copied().unwrap_or_else(|| panic!("expected ProjectError, got {err}"));
    let _ = project.close();
    host.close().unwrap();
    project_error.kind
}

// Go `&contentmapper.Mapper{Package: "dynamic", Name: "dynamic", Version: "1.0.0", Exec: []string{"mapper"}, DynamicConfig: true}`.
fn dynamic_package_mapper() -> &'static Mapper {
    let mut mapper = new_mapper("dynamic", &[], "dynamic", "1.0.0", &["mapper"]);
    mapper.manifest.dynamic_config = true;
    leak(mapper)
}

// host_test.go:1112
#[test]
fn test_project_rejects_relative_watched_files() {
    let mapper_process =
        recordingMapper { watched_files: Some(vec!["mapper.config.js".to_string()]), dynamic_config: true, ..Default::default() };
    assert_eq!(project_error_kind(mapper_process, dynamic_package_mapper()), ProjectErrorKind::NonAbsoluteWatchedFile);
}

// host_test.go:1135
#[test]
fn test_dynamic_project_requires_config_identity() {
    let mapper_process = recordingMapper { config_identity: Some(String::new()), dynamic_config: true, ..Default::default() };
    assert_eq!(project_error_kind(mapper_process, dynamic_package_mapper()), ProjectErrorKind::MissingConfigIdentity);
}

// host_test.go:1159
#[test]
fn test_static_mapper_rejects_dynamic_project_response_fields() {
    let tests = [
        (
            "config identity",
            recordingMapper { config_identity: Some("dynamic".to_string()), ..Default::default() },
            ProjectErrorKind::UnexpectedConfigIdentity,
        ),
        (
            "watched files",
            recordingMapper { watched_files: Some(vec!["/repo/mapper.config.js".to_string()]), ..Default::default() },
            ProjectErrorKind::UnexpectedWatchedFiles,
        ),
    ];
    for (name, mapper, kind) in tests {
        let project_mapper = leak(new_mapper("", &[], "static", "1.0.0", &["mapper"]));
        assert_eq!(project_error_kind(mapper, project_mapper), kind, "{name}");
    }
}

// host_test.go:1191
#[test]
fn test_project_rejects_invalid_option_diagnostic_path() {
    let mapper_process = recordingMapper {
        option_diagnostics: vec![OptionDiagnosticResult {
            path: vec![Value::Null],
            message_text: "Invalid option.".to_string(),
            code: 123,
        }],
        ..Default::default()
    };
    let host = new_host(fake_spawner(Some(Arc::new(mapper_process))), Locale::DEFAULT);
    let project_mapper = leak(new_mapper("", &[], "mapper", "1.0.0", &["mapper"]));
    let project = host
        .project(ProjectSpec {
            mappers: vec![project_mapper],
            compiler_options: compiler_options(CompilerOptions::default()),
            ..Default::default()
        })
        .unwrap();
    let err = project.transform(project_mapper, request("/repo/file.ext", "x")).unwrap_err();
    let transform_error = err.as_transform_error().unwrap_or_else(|| panic!("expected TransformError, got {err}"));
    let project_error = Error::Transform(transform_error.clone()).as_project_error().copied().unwrap();
    assert_eq!(project_error.kind, ProjectErrorKind::MalformedResponse);
    let _ = project.close();
    host.close().unwrap();
}

// host_test.go:1215 (Go `locale.Parse("cs-CZ")`: tsrs ports English only and keeps a locale's tag.)
#[test]
fn test_runner_forwards_project_options() {
    let mapper = Arc::new(recordingMapper::default());
    let diagnostic_locale = Locale("cs-CZ".to_string());
    let r = new_host(fake_spawner(Some(Arc::clone(&mapper) as Arc<dyn testMapper>)), diagnostic_locale);

    let mapper_definition = project_mapper(r#"{"strictTemplates":true}"#, "vue", &["target", "jsx"], false);
    let options = compiler_options(CompilerOptions { target: ScriptTarget::ES2020, strict: Tristate::True, ..Default::default() });
    let project = r.project(ProjectSpec { mappers: vec![mapper_definition], compiler_options: options, ..Default::default() }).unwrap();
    project.transform(mapper_definition, request("/a.vue", "x")).unwrap();

    let want = json::marshal(&compiler_options_to_go_json(&options.unwrap())).unwrap();
    {
        let state = lock(&mapper.mu);
        assert_eq!(state.received, want);
        assert_eq!(state.received_options, r#"{"strictTemplates":true}"#);
        assert_eq!(state.received_locale, "cs-CZ");
        assert!(!state.transform_params.contains(r#""options""#));
        assert!(!state.transform_params.contains(r#""compilerOptions""#));
    }
    project.close().unwrap();
    r.close().unwrap();
}

// host_test.go:1247
#[test]
fn test_host_set_locale_restarts_mapper() {
    let mapper = Arc::new(recordingMapper::default());
    let spawner = fake_spawner(Some(Arc::clone(&mapper) as Arc<dyn testMapper>));
    let r = new_host(Arc::clone(&spawner) as Arc<dyn Spawner>, Locale::DEFAULT);

    let definition = leak(new_mapper("", &[], "vue", "1.0.0", &["vue-mapper"]));
    let release = r.acquire(&[definition]);

    r.transform(definition, request("/a.vue", "x")).unwrap();
    assert_eq!(spawner.spawns.load(SeqCst), 1);

    r.set_locale(Locale("fr".to_string()));
    assert_eq!(spawner.closes.load(SeqCst), 1);

    r.transform(definition, request("/a.vue", "x")).unwrap();
    assert_eq!(spawner.spawns.load(SeqCst), 2);
    assert_eq!(lock(&mapper.mu).received_locale, "fr");
    release();
    r.close().unwrap();
}

// host_test.go:1275
#[test]
fn test_host_set_locale_waits_for_transform() {
    let (started_tx, started) = mpsc::channel::<()>();
    let (proceed, proceed_rx) = mpsc::channel::<()>();
    let mapper = Arc::new(blockingMapper {
        recording_mapper: recordingMapper::default(),
        started: Mutex::new(Some(started_tx)),
        proceed: Mutex::new(proceed_rx),
    });
    let spawner = fake_spawner(Some(mapper));
    let r = new_host(Arc::clone(&spawner) as Arc<dyn Spawner>, Locale::DEFAULT);
    let definition = leak(new_mapper("", &[], "vue", "1.0.0", &["vue-mapper"]));

    let transform_done = {
        let r = Arc::clone(&r);
        thread::spawn(move || r.transform(definition, request("/a.vue", "x")).map(|_| ()))
    };
    let _ = started.recv();

    let (set_started_tx, set_started) = mpsc::channel::<()>();
    let (set_done_tx, set_done) = mpsc::channel::<()>();
    {
        let r = Arc::clone(&r);
        thread::spawn(move || {
            drop(set_started_tx);
            r.set_locale(Locale("fr".to_string()));
            drop(set_done_tx);
        });
    }
    let _ = set_started.recv();
    let set_completed = matches!(set_done.try_recv(), Err(mpsc::TryRecvError::Disconnected));
    assert!(!set_completed, "SetLocale completed while a transform was in flight");

    drop(proceed);
    transform_done.join().unwrap().unwrap();
    let _ = set_done.recv();
    assert_eq!(spawner.closes.load(SeqCst), 1);
    r.close().unwrap();
}
