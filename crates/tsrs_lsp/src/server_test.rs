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
    let _ = server.background_ctx.set(test_ctx);

    let msg = lsproto::WINDOW_LOG_MESSAGE_INFO
        .new_notification_message(lsproto::LogMessageParams { type_: lsproto::MessageType::Info, message: "queued".to_string() })
        .message();

    let (tx, rx) = std::sync::mpsc::channel();
    let s = server;
    std::thread::spawn(move || {
        if (0..1000).any(|_| {
            if let Err(err) = s.send(msg.clone()) {
                let _ = tx.send(Err(err));
                return true;
            }
            false
        }) {
            return;
        }
        let _ = tx.send(Ok(()));
    });

    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(result) => result.unwrap(),
        Err(err) => panic!("sending outgoing messages blocked without a writer: {err}"),
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
    let writer = cancelingTestWriter { writer: to_writer(std::io::sink()), cancel, messages: std::sync::Arc::clone(&messages) };
    let server = new_server(options(Box::new(writer)));

    let _ = server.background_ctx.set(ctx.clone());

    // A selection range whose parent chain is far deeper than the JSON encoder's nesting limit.
    let mut deep: Option<Box<lsproto::SelectionRange>> = None;
    deep = (0..20000).fold(deep, |mut deep, _| {
        deep = Some(Box::new(lsproto::SelectionRange { parent: deep, ..Default::default() }));

        deep
    });
    let bad_result = vec![*deep.unwrap()];
    let bad_id = jsonrpc::new_id_string("bad");
    server
        .send(ResponseMessage { id: Some(bad_id.clone()), result: Some(bad_result.to_json()), ..Default::default() }.message())
        .unwrap_or_else(|err| panic!("failed to enqueue bad response: {err}"));
    drop(bad_result);

    // A subsequent well-formed response must still be delivered.
    let good_id = jsonrpc::new_id_string("good");
    server
        .send(ResponseMessage { id: Some(good_id.clone()), result: Some(lsproto::SelectionRangesOrNull::default().to_json()), ..Default::default() }.message())
        .unwrap_or_else(|err| panic!("failed to enqueue good response: {err}"));

    let err = server.write_loop(&ctx).unwrap_err();
    assert!(err.is(ErrorTag::ContextCanceled), "write loop exited unexpectedly: {err}");
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
    opts.fs = Some(std::sync::Arc::clone(&fs));
    opts.default_library_path = tsrs_vfs::bundled::lib_path();
    let server = new_server(opts);

    let (ctx, cancel) = Context::background().with_cancel();
    let _ = server.background_ctx.set(ctx.clone());

    // Start write loop to drain queue
    let s = std::sync::Arc::clone(&server);
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
    server.set_session_for_test(std::sync::Arc::clone(&session));

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

// Not a Go test. completionItem/resolve takes the byte offset in `data.position` from the client unconverted; one
// inside a multi-byte character (here in a JSDoc comment) resolves like the character's start instead of panicking
// while slicing the text there; one past the end of the file resolves as in Go.
#[test]
fn completion_resolve_snaps_client_data_position() {
    use crate::lsptestutil::{acknowledge_registrations, new_lsp_client, test_server_options};

    let text = "/** 日本 ü */\nconst abc = 1;\nab\n";
    let (client, close_client) = new_lsp_client(test_server_options("/home/projects", &[]), Some(acknowledge_registrations()));
    let (init_msg, _) = client.send_request(lsproto::INITIALIZE_INFO, lsproto::InitializeParams::default());
    assert!(init_msg.error.is_none(), "Initialize failed");
    client.send_notification(lsproto::INITIALIZED_INFO, lsproto::InitializedParams::default());
    client.server.init_complete().wait();
    let uri = lsproto::DocumentUri::from("file:///home/projects/index.ts");
    client.send_notification(
        lsproto::TEXT_DOCUMENT_DID_OPEN_INFO,
        lsproto::DidOpenTextDocumentParams {
            text_document: lsproto::TextDocumentItem { uri: uri.clone(), language_id: lsproto::LanguageKind::TypeScript, text: text.to_string(), ..Default::default() },
        },
    );
    let (msg, list) = client.send_request(
        lsproto::TEXT_DOCUMENT_COMPLETION_INFO,
        lsproto::CompletionParams { text_document: lsproto::TextDocumentIdentifier { uri }, position: lsproto::Position { line: 2, character: 2 }, ..Default::default() },
    );
    assert!(msg.error.is_none(), "{:?}", msg.error);
    let list = list.unwrap();
    let items = list.list.map(|l| l.items).or(list.items).unwrap();
    let item = items.into_iter().find(|i| i.label == "abc").expect("abc completion");
    let inside_character = text.find('日').unwrap() + 1;
    for position in [inside_character, inside_character + 1, text.find('ü').unwrap() + 1, text.len() + 100] {
        let mut item = item.clone();
        item.data.as_mut().unwrap().position = i32::try_from(position).unwrap();
        let (msg, _) = client.send_request(lsproto::COMPLETION_ITEM_RESOLVE_INFO, item);
        assert!(msg.error.is_none(), "position {position}: {:?}", msg.error);
    }
    let _ = close_client.close();
}

// Not a Go test (Go panics on each, ending the session). A client's bad input gets an error, and the server keeps
// answering: URIs `file_name` cannot use (in document requests and notifications, file renames, watched file events
// and call hierarchy items), a change after the close of the same document, a completion item without a rooted file
// name and a code lens without data.
#[test]
fn bad_client_input_does_not_end_the_session() {
    use crate::lsptestutil::{acknowledge_registrations, new_lsp_client, test_server_options};

    let files = [("/home/projects/tsconfig.json", "{}"), ("/home/projects/index.ts", "export const abc = 1;\n")];
    let (client, close_client) = new_lsp_client(test_server_options("/home/projects", &files), Some(acknowledge_registrations()));
    let (init_msg, _) = client.send_request(lsproto::INITIALIZE_INFO, lsproto::InitializeParams::default());
    assert!(init_msg.error.is_none(), "Initialize failed");
    client.send_notification(lsproto::INITIALIZED_INFO, lsproto::InitializedParams::default());
    client.server.init_complete().wait();
    let good = lsproto::DocumentUri::from("file:///home/projects/index.ts");
    let hover = |uri: &lsproto::DocumentUri| {
        client.send_request(
            lsproto::TEXT_DOCUMENT_HOVER_INFO,
            lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: lsproto::Position { line: 0, character: 14 }, ..Default::default() },
        )
    };
    let open = |uri: &lsproto::DocumentUri| {
        client.send_notification(
            lsproto::TEXT_DOCUMENT_DID_OPEN_INFO,
            lsproto::DidOpenTextDocumentParams {
                text_document: lsproto::TextDocumentItem { uri: uri.clone(), language_id: lsproto::LanguageKind::TypeScript, text: "export const abc = 1;\n".to_string(), ..Default::default() },
            },
        );
    };
    let is_invalid_params = |msg: &lsproto::ResponseMessage| msg.error.as_ref().is_some_and(|e| e.code == lsproto::ErrorCode::InvalidParams.0);
    open(&good);

    for bad in ["", "not a uri", "relative/path.ts", "file://", "foo://host"] {
        let uri = lsproto::DocumentUri::from(bad);
        let (msg, _) = hover(&uri);
        assert!(is_invalid_params(&msg), "hover {bad:?}: {:?}", msg.error);
        open(&uri);
        client.send_notification(lsproto::TEXT_DOCUMENT_DID_CLOSE_INFO, lsproto::DidCloseTextDocumentParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() } });
        client.send_notification(
            lsproto::WORKSPACE_DID_CHANGE_WATCHED_FILES_INFO,
            lsproto::DidChangeWatchedFilesParams { changes: vec![lsproto::FileEvent { uri: uri.clone(), type_: lsproto::FileChangeType(2) }] },
        );
        let (msg, _) = client.send_request(
            lsproto::WORKSPACE_WILL_RENAME_FILES_INFO,
            lsproto::RenameFilesParams { files: vec![lsproto::FileRename { old_uri: uri.clone(), new_uri: good.clone() }] },
        );
        assert!(is_invalid_params(&msg), "willRenameFiles {bad:?}: {:?}", msg.error);
        let item = lsproto::CallHierarchyItem { name: "abc".to_string(), uri: uri.clone(), ..Default::default() };
        let (msg, _) = client.send_request(lsproto::CALL_HIERARCHY_OUTGOING_CALLS_INFO, lsproto::CallHierarchyOutgoingCallsParams { item, ..Default::default() });
        assert!(is_invalid_params(&msg), "outgoingCalls {bad:?}: {:?}", msg.error);
    }

    // A change right after the close of the same document.
    let other = lsproto::DocumentUri::from("file:///home/projects/other.ts");
    open(&other);
    client.send_notification(lsproto::TEXT_DOCUMENT_DID_CLOSE_INFO, lsproto::DidCloseTextDocumentParams { text_document: lsproto::TextDocumentIdentifier { uri: other.clone() } });
    client.send_notification(
        lsproto::TEXT_DOCUMENT_DID_CHANGE_INFO,
        lsproto::DidChangeTextDocumentParams {
            text_document: lsproto::VersionedTextDocumentIdentifier { uri: other, version: 2 },
            content_changes: vec![lsproto::TextDocumentContentChangePartialOrWholeDocument {
                whole_document: Some(lsproto::TextDocumentContentChangeWholeDocument { text: "x".to_string() }),
                ..Default::default()
            }],
        },
    );

    for file_name in ["", "tsconfig.json", "untitled:Untitled-1", "file:///home/projects/index.ts"] {
        let item = lsproto::CompletionItem {
            label: "abc".to_string(),
            data: Some(lsproto::CompletionItemData { file_name: file_name.to_string(), ..Default::default() }),
            ..Default::default()
        };
        let (msg, _) = client.send_request(lsproto::COMPLETION_ITEM_RESOLVE_INFO, item);
        assert!(is_invalid_params(&msg), "resolve {file_name:?}: {:?}", msg.error);
    }
    let (msg, _) = client.send_request(lsproto::CODE_LENS_RESOLVE_INFO, lsproto::CodeLens::default());
    assert!(is_invalid_params(&msg), "code lens without data: {:?}", msg.error);

    let (msg, resp) = hover(&good);
    assert!(msg.error.is_none() && resp.is_some(), "the session still answers: {:?}", msg.error);
    let _ = close_client.close();
}

// Not a Go test (Go dereferences the missing session and panics). A document request between `initialize` and
// `initialized` gets ServerNotInitialized.
#[test]
fn document_requests_before_initialized_get_an_error() {
    use crate::lsptestutil::{acknowledge_registrations, new_lsp_client, test_server_options};

    let (client, close_client) = new_lsp_client(test_server_options("/home/projects", &[]), Some(acknowledge_registrations()));
    let (init_msg, _) = client.send_request(lsproto::INITIALIZE_INFO, lsproto::InitializeParams::default());
    assert!(init_msg.error.is_none(), "Initialize failed");
    let uri = lsproto::DocumentUri::from("file:///home/projects/index.ts");
    let (msg, _) = client.send_request(
        lsproto::TEXT_DOCUMENT_HOVER_INFO,
        lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri }, ..Default::default() },
    );
    assert_eq!(msg.error.map(|e| e.code), Some(lsproto::ErrorCode::ServerNotInitialized.0));
    let _ = close_client.close();
}

// Not a Go test (Go panics in `FileName` when the session starts). A rootUri the server cannot use falls back to the
// server's directory.
#[test]
fn a_bad_root_uri_falls_back_to_the_server_directory() {
    use crate::lsptestutil::{acknowledge_registrations, new_lsp_client, test_server_options};

    let files = [("/home/projects/tsconfig.json", "{}"), ("/home/projects/index.ts", "export const abc = 1;\n")];
    let (client, close_client) = new_lsp_client(test_server_options("/home/projects", &files), Some(acknowledge_registrations()));
    let params = lsproto::InitializeParams { root_uri: lsproto::DocumentUriOrNull { document_uri: Some(lsproto::DocumentUri::from("not a uri")) }, ..Default::default() };
    let (init_msg, _) = client.send_request(lsproto::INITIALIZE_INFO, params);
    assert!(init_msg.error.is_none(), "Initialize failed");
    client.send_notification(lsproto::INITIALIZED_INFO, lsproto::InitializedParams::default());
    client.server.init_complete().wait();
    let uri = lsproto::DocumentUri::from("file:///home/projects/index.ts");
    client.send_notification(
        lsproto::TEXT_DOCUMENT_DID_OPEN_INFO,
        lsproto::DidOpenTextDocumentParams {
            text_document: lsproto::TextDocumentItem { uri: uri.clone(), language_id: lsproto::LanguageKind::TypeScript, text: "export const abc = 1;\n".to_string(), ..Default::default() },
        },
    );
    let (msg, resp) = client.send_request(
        lsproto::TEXT_DOCUMENT_HOVER_INFO,
        lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri }, position: lsproto::Position { line: 0, character: 14 }, ..Default::default() },
    );
    assert!(msg.error.is_none() && resp.is_some(), "{:?}", msg.error);
    let _ = close_client.close();
}

// Not a Go test. `workspace/willRenameFiles` sends a `documentChanges` workspace edit with one entry per changed file or
// renamed companion file; returns (uri, edit text) for each text edit and (old uri, new uri) for each file rename.
fn will_rename_files(files: &[(&str, &str)], renames: &[(&str, &str)]) -> (Vec<(String, String)>, Vec<(String, String)>) {
    use crate::lsptestutil::{acknowledge_registrations, new_lsp_client, test_server_options};

    let (client, close_client) = new_lsp_client(test_server_options("/home/projects", files), Some(acknowledge_registrations()));
    let capabilities = lsproto::ClientCapabilities {
        workspace: Some(lsproto::WorkspaceClientCapabilities {
            workspace_edit: Some(lsproto::WorkspaceEditClientCapabilities { document_changes: Some(true), ..Default::default() }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let (init_msg, _) = client.send_request(lsproto::INITIALIZE_INFO, lsproto::InitializeParams { capabilities, ..Default::default() });
    assert!(init_msg.error.is_none(), "Initialize failed");
    client.send_notification(lsproto::INITIALIZED_INFO, lsproto::InitializedParams::default());
    client.server.init_complete().wait();
    let (file_name, text) = files.iter().find(|(name, _)| std::path::Path::new(name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("ts"))).unwrap();
    client.send_notification(
        lsproto::TEXT_DOCUMENT_DID_OPEN_INFO,
        lsproto::DidOpenTextDocumentParams {
            text_document: lsproto::TextDocumentItem {
                uri: lsproto::DocumentUri::from(format!("file://{file_name}")),
                language_id: lsproto::LanguageKind::TypeScript,
                text: text.to_string(),
                ..Default::default()
            },
        },
    );
    let params = lsproto::RenameFilesParams {
        files: renames
            .iter()
            .map(|(old, new)| lsproto::FileRename { old_uri: lsproto::DocumentUri::from(format!("file://{old}")), new_uri: lsproto::DocumentUri::from(format!("file://{new}")) })
            .collect(),
    };
    let (msg, resp) = client.send_request(lsproto::WORKSPACE_WILL_RENAME_FILES_INFO, params);
    assert!(msg.error.is_none(), "{:?}", msg.error);
    let _ = close_client.close();
    let mut edits = Vec::new();
    let mut file_renames = Vec::new();
    for change in resp.and_then(|r| r.workspace_edit).and_then(|e| e.document_changes).unwrap_or_default() {
        if let Some(rename) = change.rename_file {
            file_renames.push((rename.old_uri.to_string(), rename.new_uri.to_string()));
        }
        if let Some(text_document_edit) = change.text_document_edit {
            let uri = text_document_edit.text_document.uri.to_string();
            edits.extend(text_document_edit.edits.into_iter().map(|edit| (uri.clone(), edit.text_edit.expect("a plain text edit").new_text)));
        }
    }
    (edits, file_renames)
}

// Not a Go test (Go rewrites every unresolved relative import, here `../../../x` to `../../x`, since the path climbs
// past the root). A file whose unresolved imports point at nothing renamed gets no edit.
#[test]
fn will_rename_files_leaves_unrelated_unresolved_imports_alone() {
    let files = [
        ("/home/projects/tsconfig.json", "{}"),
        ("/home/projects/a.ts", "export const a = 1;\n"),
        ("/home/projects/b.ts", "import { a } from \"./a\";\nimport { x } from \"../../../x\";\nimport { y } from \"./missing\";\n"),
        ("/home/projects/c.ts", "/// <reference path=\"../../../y.d.ts\" />\nimport { x } from \"../../../x\";\n"),
    ];
    let (edits, _) = will_rename_files(&files, &[("/home/projects/a.ts", "/home/projects/a2.ts")]);
    assert_eq!(edits, vec![("file:///home/projects/b.ts".to_string(), "./a2".to_string())]);
}

// Not a Go test (Go computes each rename on its own, so `./b` in a.ts got both `../src/b` and `../lib/b` here). The
// renames of a file and a file it imports are computed together.
#[test]
fn will_rename_files_computes_several_renames_together() {
    let files = [
        ("/home/projects/tsconfig.json", "{}"),
        ("/home/projects/src/a.ts", "import { b } from \"./b\";\nimport { c } from \"./c\";\n"),
        ("/home/projects/src/b.ts", "export const b = 1;\n"),
        ("/home/projects/src/c.ts", "export const c = 1;\n"),
        ("/home/projects/main.ts", "import { b } from \"./src/b\";\nimport { a } from \"./src/a\";\n"),
    ];
    let (mut edits, _) = will_rename_files(
        &files,
        &[("/home/projects/src/a.ts", "/home/projects/lib/a.ts"), ("/home/projects/src/b.ts", "/home/projects/lib/b.ts")],
    );
    edits.sort();
    let edit = |file: &str, text: &str| (format!("file:///home/projects/{file}"), text.to_string());
    assert_eq!(edits, vec![edit("main.ts", "./lib/a"), edit("main.ts", "./lib/b"), edit("src/a.ts", "../src/c")]);
}

// Not a Go test (Go took the new declaration file's extension from the old name, so `foo.css` was renamed to
// `bar.css` here though bar.d.ts is not a declaration for a .css file).
#[test]
fn will_rename_files_renames_the_companion_only_for_a_matching_declaration() {
    let files = [
        ("/home/projects/tsconfig.json", "{}"),
        ("/home/projects/index.ts", "export {};\n"),
        ("/home/projects/foo.d.css.ts", "export {};\n"),
        ("/home/projects/foo.css", ""),
    ];
    let (_, renames) = will_rename_files(&files, &[("/home/projects/foo.d.css.ts", "/home/projects/bar.d.ts")]);
    assert_eq!(renames, Vec::<(String, String)>::new());
    let (_, renames) = will_rename_files(&files, &[("/home/projects/foo.d.css.ts", "/home/projects/bar.d.css.ts")]);
    assert_eq!(renames, vec![("file:///home/projects/foo.css".to_string(), "file:///home/projects/bar.css".to_string())]);
}
