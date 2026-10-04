use std::sync::{Arc, Mutex};

use tsrs_core::context::{CancelFunc, Context, ContextError};
use tsrs_lsproto as lsproto;
use tsrs_lsproto::jsonrpc;
use tsrs_lsproto::{Error, ErrorCode, ErrorTag, Json, Message, ResponseMessage};

use crate::server::{new_server, to_writer, Reader, ServerOptions, Writer};

// server_test.go:16
struct shutdownTestReader;

impl Reader for shutdownTestReader {
    fn read(&mut self) -> (Option<Message>, Option<Error>) {
        (None, Some(Error::tagged(ErrorTag::EOF, "EOF")))
    }
}

// server_test.go:20
struct shutdownTestWriter;

impl Writer for shutdownTestWriter {
    fn write(&mut self, _msg: &Message) -> Result<(), Error> {
        Ok(())
    }
}

// server_test.go:24
struct cancelingTestWriter {
    writer: Box<dyn Writer>,
    cancel: CancelFunc,
    messages: Arc<Mutex<Vec<Message>>>,
}

impl Writer for cancelingTestWriter {
    fn write(&mut self, msg: &Message) -> Result<(), Error> {
        self.writer.write(msg)?;
        let mut messages = self.messages.lock().unwrap();
        messages.push(msg.clone());
        if messages.len() == 2 {
            self.cancel.call();
        }
        Ok(())
    }
}

fn options(out: Box<dyn Writer>) -> ServerOptions {
    ServerOptions {
        in_: Box::new(shutdownTestReader),
        out,
        err: Box::new(std::io::sink()),
        cwd: "/test".to_string(),
        fs: None,
        default_library_path: String::new(),
        typings_location: String::new(),
        parse_cache: None,
        npm_install: None,
        progress_delay: std::time::Duration::ZERO,
        set_parent_process_id: None,
    }
}

// server_test.go:115
#[test]
fn test_server_outgoing_queue_does_not_block_without_writer() {
    let server = new_server(options(Box::new(shutdownTestWriter)));
    let (test_ctx, cancel) = Context::background().with_cancel();
    let _ = server.background_ctx.set(test_ctx.clone());

    let msg = lsproto::WINDOW_LOG_MESSAGE_INFO
        .new_notification_message(lsproto::LogMessageParams { type_: lsproto::MessageType::Info, message: "queued".to_string() })
        .message();

    let (tx, rx) = std::sync::mpsc::channel();
    let s = server.clone();
    std::thread::spawn(move || {
        for _ in 0..1000 {
            if let Err(err) = s.send(msg.clone()) {
                let _ = tx.send(Err(err));
                return;
            }
        }
        let _ = tx.send(Ok(()));
    });

    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(result) => result.unwrap(),
        Err(_) => panic!("sending outgoing messages blocked without a writer"),
    }
    cancel.call();
}

// server_test.go:149
// A response that exceeds the JSON encoder's nesting limit must fail only its
// request. The write loop must remain available to deliver subsequent responses.
#[test]
fn test_write_loop_recovers_from_unserializable_response() {
    // The 20000-deep value is built, encoded and dropped recursively: run on a thread with a large stack.
    std::thread::Builder::new().stack_size(256 << 20).spawn(write_loop_recovers_from_unserializable_response).unwrap().join().unwrap();
}

fn write_loop_recovers_from_unserializable_response() {
    let (ctx, cancel) = Context::background().with_cancel();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let writer = cancelingTestWriter { writer: to_writer(std::io::sink()), cancel: cancel.clone(), messages: messages.clone() };
    let server = new_server(options(Box::new(writer)));

    let _ = server.background_ctx.set(ctx.clone());

    // A selection range whose parent chain is far deeper than the JSON encoder's nesting limit.
    let mut deep: Option<Box<lsproto::SelectionRange>> = None;
    for _ in 0..20000 {
        deep = Some(Box::new(lsproto::SelectionRange { parent: deep, ..Default::default() }));
    }
    let bad_result = vec![*deep.unwrap()];
    let bad_id = jsonrpc::new_id_string("bad");
    server
        .send(ResponseMessage { id: Some(bad_id.clone()), result: Some(bad_result.to_json()), ..Default::default() }.message())
        .unwrap_or_else(|err| panic!("failed to enqueue bad response: {}", err));
    drop(bad_result);

    // A subsequent well-formed response must still be delivered.
    let good_id = jsonrpc::new_id_string("good");
    server
        .send(ResponseMessage { id: Some(good_id.clone()), result: Some(lsproto::SelectionRangesOrNull::default().to_json()), ..Default::default() }.message())
        .unwrap_or_else(|err| panic!("failed to enqueue good response: {}", err));

    let err = server.write_loop(&ctx).unwrap_err();
    assert!(err.is(ErrorTag::ContextCanceled), "write loop exited unexpectedly: {}", err);
    assert_eq!(ctx.err(), Some(ContextError::Canceled));

    let mut saw_error = false;
    let mut saw_good = false;
    for msg in messages.lock().unwrap().iter() {
        let resp = msg.as_response();
        if resp.id.as_ref() == Some(&bad_id) {
            match &resp.error {
                None => panic!("expected an error response for the unserializable request, got a result"),
                Some(error) => assert_eq!(error.code, ErrorCode::InternalError.0, "error response code = {}, want {}", error.code, ErrorCode::InternalError.0),
            }
            saw_error = true;
        } else if resp.id.as_ref() == Some(&good_id) {
            assert!(resp.error.is_none(), "expected a successful response for the good request, got error: {:?}", resp.error);
            saw_good = true;
        } else {
            panic!("unexpected response id: {:?}", resp.id);
        }
    }

    assert!(saw_error, "did not receive an error response for the unserializable request");
    assert!(saw_good, "did not receive the subsequent well-formed response (write loop likely died)");
}

// server_test.go:41
// TestServerShutdownNoDeadlock verifies that operations after shutdown
// don't block.
#[test]
fn test_server_shutdown_no_deadlock() {
    if !tsrs_vfs::bundled::EMBEDDED {
        eprintln!("bundled files are not embedded");
        return;
    }
    // Programs are built on this thread (parse + bind): use a large stack like the server threads.
    std::thread::Builder::new().stack_size(256 << 20).spawn(server_shutdown_no_deadlock).unwrap().join().unwrap();
}

fn server_shutdown_no_deadlock() {
    use tsrs_project as project;

    let fs: Arc<dyn tsrs_vfs::FS> = Arc::new(tsrs_vfs::bundled::wrap_fs(tsrs_vfs::vfstest::from_map(
        [("/test/tsconfig.json", "{}"), ("/test/index.ts", "const x = 1;")],
        false,
    )));

    let mut opts = options(Box::new(shutdownTestWriter));
    opts.fs = Some(fs.clone());
    opts.default_library_path = tsrs_vfs::bundled::lib_path();
    let server = new_server(opts);

    let (ctx, cancel) = Context::background().with_cancel();
    let _ = server.background_ctx.set(ctx.clone());

    // Start write loop to drain queue
    let s = server.clone();
    let write_ctx = ctx.clone();
    let write_loop_done = std::thread::spawn(move || {
        let _ = s.write_loop(&write_ctx);
    });

    // Create session with the server's lifecycle context
    server.init_started.store(true, std::sync::atomic::Ordering::SeqCst);
    let logger: Arc<dyn project::logging::Logger> = server.logger_arc();
    let session = project::new_session(&project::SessionInit {
        background_ctx: ctx.clone(),
        options: Arc::new(project::SessionOptions {
            current_directory: "/test".to_string(),
            default_library_path: tsrs_vfs::bundled::lib_path(),
            position_encoding: lsproto::PositionEncodingKind::UTF8,
            watch_enabled: false,
            logging_enabled: true,
            ..Default::default()
        }),
        fs,
        client: None,
        logger: Some(logger),
        npm_executor: None,
        parse_cache: None,
        content_mapped_parse_cache: None,
    });
    server.set_session_for_test(session.clone());

    // Open a file to establish a project
    session.did_open_file(&ctx, "file:///test/index.ts".into(), 1, "const x = 1;".to_string(), lsproto::LanguageKind::TypeScript);
    session.wait_for_background_tasks();

    // Shutdown (cancel context and wait for write loop to exit)
    cancel.call();
    write_loop_done.join().unwrap();

    // Trigger operations that would log (these should not block)
    session.did_change_file(
        &ctx,
        "file:///test/index.ts".into(),
        2,
        vec![lsproto::TextDocumentContentChangePartialOrWholeDocument {
            whole_document: Some(lsproto::TextDocumentContentChangeWholeDocument { text: "const x = 2;".to_string() }),
            ..Default::default()
        }],
    );
    let _ = session.get_language_service(&ctx, &"file:///test/index.ts".into());
    session.wait_for_background_tasks();

    session.close();
}
