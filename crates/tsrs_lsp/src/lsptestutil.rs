// Port of testutil/lsptestutil/lspclient.go (the in-process LSP test client), as a test module of this crate.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use rustc_hash::FxHashMap;
use tsrs_core::context::{CancelFunc, Context};
use tsrs_lsproto as lsproto;
use tsrs_lsproto::jsonrpc::{self, MessageKind, ID};
use tsrs_lsproto::{Error, ErrorTag, Json, Message, RequestMessage, ResponseMessage};

use crate::server::{new_server, to_reader, to_writer, Reader, Server, ServerOptions, Writer};

// Go's io.Pipe (buffered here: writes do not wait for the reader).
#[derive(Default)]
struct pipeState {
    buf: VecDeque<u8>,
    closed: bool,
}

#[derive(Clone, Default)]
struct pipe(Arc<(Mutex<pipeState>, Condvar)>);

impl pipe {
    fn close(&self) {
        self.0 .0.lock().unwrap().closed = true;
        self.0 .1.notify_all();
    }
}

impl Read for pipe {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let mut state = self.0 .0.lock().unwrap();
        while state.buf.is_empty() && !state.closed {
            state = self.0 .1.wait(state).unwrap();
        }
        let n = out.len().min(state.buf.len());
        for (i, b) in state.buf.drain(..n).enumerate() {
            out[i] = b;
        }
        Ok(n)
    }
}

impl Write for pipe {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut state = self.0 .0.lock().unwrap();
        if state.closed {
            return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "io: read/write on closed pipe"));
        }
        state.buf.extend(data);
        drop(state);
        self.0 .1.notify_all();
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// lspclient.go:39
// clientTransport wires a test client to a server using real LSP
// "Content-Length"-framed JSON streamed over byte pipes, exactly like
// communication with a real editor.
struct clientTransport {
    server_in: Box<dyn Reader>,
    server_out: Box<dyn Writer>,
    client_in: Box<dyn Reader>,
    client_out: Box<dyn Writer>,
    client_to_server: pipe,
    server_to_client: pipe,
}

fn new_client_transport() -> clientTransport {
    let client_to_server = pipe::default();
    let server_to_client = pipe::default();
    clientTransport {
        server_in: to_reader(client_to_server.clone()),
        server_out: to_writer(server_to_client.clone()),
        client_in: to_reader(server_to_client.clone()),
        client_out: to_writer(client_to_server.clone()),
        client_to_server,
        server_to_client,
    }
}

// lspclient.go:53
// ServerRequestHandler handles server-initiated requests and returns the response to send back.
pub(crate) type ServerRequestHandler = Box<dyn Fn(&RequestMessage) -> Option<ResponseMessage> + Send + Sync>;

// lspclient.go:56
// ServerNotificationHandler handles server-initiated notifications (e.g., $/progress).
pub(crate) type ServerNotificationHandler = Box<dyn Fn(&RequestMessage) + Send + Sync>;

// lspclient.go:59
pub(crate) struct LSPClient {
    pub(crate) server: Arc<Server>,
    input_writer: Mutex<Box<dyn Writer>>,
    id: AtomicI32,
    ctx: Context,
    on_server_request: Option<ServerRequestHandler>,
    pub(crate) on_server_notification: Mutex<Option<ServerNotificationHandler>>,
    pending_requests: Mutex<FxHashMap<ID, std::sync::mpsc::SyncSender<ResponseMessage>>>,
}

pub(crate) struct closeClient {
    cancel: CancelFunc,
    client_to_server: pipe,
    threads: Vec<JoinHandle<Result<(), Error>>>,
}

impl closeClient {
    // lspclient.go:117
    pub(crate) fn close(self) -> Result<(), Error> {
        self.cancel.call();
        self.client_to_server.close();
        let mut first = None;
        for t in self.threads {
            if let Err(err) = t.join().unwrap() {
                first.get_or_insert(err);
            }
        }
        match first {
            Some(err) if !err.is(ErrorTag::ContextCanceled) => Err(err),
            _ => Ok(()),
        }
    }
}

// lspclient.go:93
// NewLSPClient creates an LSPClient wrapping the given server and pipes.
pub(crate) fn new_lsp_client(mut server_opts: ServerOptions, on_server_request: Option<ServerRequestHandler>) -> (Arc<LSPClient>, closeClient) {
    let transport = new_client_transport();
    server_opts.in_ = transport.server_in;
    server_opts.out = transport.server_out;

    let server = new_server(server_opts);

    let (ctx, cancel) = Context::background().with_cancel();
    let client = Arc::new(LSPClient {
        server: server.clone(),
        input_writer: Mutex::new(transport.client_out),
        id: AtomicI32::new(0),
        ctx: ctx.clone(),
        on_server_request,
        on_server_notification: Mutex::new(None),
        pending_requests: Mutex::new(FxHashMap::default()),
    });

    let mut threads = Vec::new();
    // Start server goroutine
    {
        let ctx = ctx.clone();
        let server_to_client = transport.server_to_client.clone();
        threads.push(std::thread::spawn(move || {
            let result = server.run(&ctx);
            server_to_client.close();
            result
        }));
    }
    // Start async message router
    {
        let client = client.clone();
        let ctx = ctx.clone();
        let mut output_reader = transport.client_in;
        threads.push(std::thread::spawn(move || client.message_router(&ctx, &mut *output_reader)));
    }

    (client, closeClient { cancel, client_to_server: transport.client_to_server, threads })
}

impl LSPClient {
    // lspclient.go:83
    fn write_to_server(&self, msg: &Message) -> Result<(), Error> {
        self.input_writer.lock().unwrap().write(msg)
    }

    // lspclient.go:128
    fn next_id(&self) -> i32 {
        self.id.fetch_add(1, Ordering::SeqCst)
    }

    // lspclient.go:138
    fn message_router(&self, ctx: &Context, output_reader: &mut dyn Reader) -> Result<(), Error> {
        loop {
            let (msg, err) = output_reader.read();
            if let Some(err) = err {
                if err.is_eof() {
                    return Ok(());
                }
                if ctx.err().is_some() {
                    return Ok(());
                }
                return Err(Error::wrap("failed to read message: ", err));
            }
            let msg = msg.unwrap();

            // After context cancellation, keep draining but don't process messages.
            if ctx.err().is_some() {
                continue;
            }

            match msg.kind {
                MessageKind::Response => self.handle_response(msg.as_response()),
                MessageKind::Request => self.handle_server_request(ctx, msg.as_request())?,
                MessageKind::Notification => {
                    if let Some(f) = &*self.on_server_notification.lock().unwrap() {
                        f(msg.as_request());
                    }
                }
            }
        }
    }

    // lspclient.go:181
    fn handle_response(&self, resp: &ResponseMessage) {
        let Some(id) = &resp.id else {
            return;
        };
        let resp_chan = self.pending_requests.lock().unwrap().remove(id);
        if let Some(resp_chan) = resp_chan {
            let _ = resp_chan.send(resp.clone());
        }
    }

    // lspclient.go:205
    fn handle_server_request(&self, ctx: &Context, req: &RequestMessage) -> Result<(), Error> {
        let mut response = None;
        if let Some(f) = &self.on_server_request {
            response = f(req);
        }
        let response = response.unwrap_or_else(|| ResponseMessage {
            id: req.id.clone(),
            error: Some(jsonrpc::ResponseError { code: lsproto::ErrorCode::MethodNotFound.0, message: format!("Unknown method: {}", req.method), data: None }),
            ..Default::default()
        });

        if ctx.err().is_some() {
            return Ok(());
        }

        if let Err(err) = self.write_to_server(&response.message()) {
            if ctx.err().is_some() {
                return Ok(());
            }
            return Err(Error::wrap("failed to write server request response: ", err));
        }
        Ok(())
    }

    // lspclient.go:245
    pub(crate) fn write_msg(&self, msg: &Message) {
        self.write_to_server(msg).unwrap_or_else(|err| panic!("failed to write message: {}", err));
    }

    // lspclient.go:253
    // SendRequest sends a typed request and waits for the response.
    pub(crate) fn send_request<Params: Json, Resp: Json>(&self, info: lsproto::RequestInfo<Params, Resp>, params: Params) -> (ResponseMessage, Option<Resp>) {
        let id = self.next_id();
        let req_id = lsproto::new_id(&lsproto::IntegerOrString { integer: Some(id), ..Default::default() });
        let req = info.new_request_message(Some(req_id.clone()), params);

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.pending_requests.lock().unwrap().insert(req_id, tx);
        self.write_msg(&req.message());

        let resp = rx.recv_timeout(Duration::from_secs(120)).expect("Request cancelled: no response");
        // The result arrives as a raw value; decode it into Resp.
        let result = info.unmarshal_result(resp.result.as_ref()).ok();
        (resp, result)
    }

    // lspclient.go:316
    // SendNotification sends a typed notification.
    pub(crate) fn send_notification<Params: Json>(&self, info: lsproto::NotificationInfo<Params>, params: Params) {
        self.write_msg(&info.new_notification_message(params).message());
    }
}

// The `onServerRequest` handler of the lsp tests: acknowledge registrations and progress tokens.
pub(crate) fn acknowledge_registrations() -> ServerRequestHandler {
    Box::new(|req: &RequestMessage| {
        use lsproto::Method;
        match req.method {
            Method::ClientRegisterCapability | Method::ClientUnregisterCapability | Method::WindowWorkDoneProgressCreate => {
                Some(ResponseMessage { id: req.id.clone(), result: Some(lsproto::Null.to_json()), ..Default::default() })
            }
            _ => None,
        }
    })
}

pub(crate) fn test_server_options(cwd: &str, files: &[(&str, &str)]) -> ServerOptions {
    let fs: Arc<dyn tsrs_vfs::FS> = Arc::new(tsrs_vfs::bundled::wrap_fs(tsrs_vfs::vfstest::from_map(files.iter().copied(), false)));
    ServerOptions {
        in_: Box::new(nullReader),
        out: Box::new(nullWriter),
        err: Box::new(std::io::sink()),
        cwd: cwd.to_string(),
        fs: Some(fs),
        default_library_path: tsrs_vfs::bundled::lib_path(),
        typings_location: String::new(),
        parse_cache: None,
        npm_install: None,
        progress_delay: Duration::ZERO,
        set_parent_process_id: None,
    }
}

struct nullReader;

impl Reader for nullReader {
    fn read(&mut self) -> (Option<Message>, Option<Error>) {
        (None, Some(Error::tagged(ErrorTag::EOF, "EOF")))
    }
}

struct nullWriter;

impl Writer for nullWriter {
    fn write(&mut self, _msg: &Message) -> Result<(), Error> {
        Ok(())
    }
}
