use std::sync::{Arc, Mutex};
use std::time::Duration;

use tsrs_lsproto as lsproto;

use crate::lsptestutil::{acknowledge_registrations, new_lsp_client, test_server_options};

// server_progress_test.go:17
#[test]
fn test_progress_notifications_end_to_end() {
    if !tsrs_vfs::bundled::EMBEDDED {
        return;
    }

    let files = [("/home/projects/tsconfig.json", "{}"), ("/home/projects/index.ts", "export const x = 1;")];

    // Collect $/progress notifications. Signal when "end" arrives.
    let progress_notifications: Arc<Mutex<Vec<lsproto::ProgressParams>>> = Arc::default();
    let (end_tx, end_rx) = std::sync::mpsc::sync_channel::<()>(1);

    let (client, close_client) = new_lsp_client(test_server_options("/home/projects", &files), Some(acknowledge_registrations()));

    {
        let progress_notifications = progress_notifications.clone();
        *client.on_server_notification.lock().unwrap() = Some(Box::new(move |req: &lsproto::RequestMessage| {
            if req.method == lsproto::Method::Progress {
                if let Ok(params) = req.unmarshal_params::<lsproto::ProgressParams>() {
                    let is_end = params.value.end.is_some();
                    progress_notifications.lock().unwrap().push(params);
                    if is_end {
                        // Signaled, or already signaled.
                        let _ = end_tx.try_send(());
                    }
                }
            }
        }));
    }

    let (init_msg, _) = client.send_request(
        lsproto::INITIALIZE_INFO,
        lsproto::InitializeParams {
            capabilities: lsproto::ClientCapabilities {
                window: Some(lsproto::WindowClientCapabilities { work_done_progress: Some(true), ..Default::default() }),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(init_msg.error.is_none(), "Initialize failed");
    client.send_notification(lsproto::INITIALIZED_INFO, lsproto::InitializedParams::default());
    client.server.init_complete().wait();

    let uri = lsproto::DocumentUri::from("file:///home/projects/index.ts");
    client.send_notification(
        lsproto::TEXT_DOCUMENT_DID_OPEN_INFO,
        lsproto::DidOpenTextDocumentParams {
            text_document: lsproto::TextDocumentItem {
                uri: uri.clone(),
                language_id: lsproto::LanguageKind::TypeScript,
                text: "export const x = 1;".to_string(),
                ..Default::default()
            },
        },
    );

    // Send a request to ensure the server has processed the didOpen and loaded the project.
    let (msg, resp) = client.send_request(lsproto::CUSTOM_PROJECT_INFO_INFO, lsproto::ProjectInfoParams { text_document: lsproto::TextDocumentIdentifier { uri } });
    assert!(msg.error.is_none());
    assert_eq!(resp.expect("expected a response").config_file_path, "/home/projects/tsconfig.json");

    // Wait for the "end" progress notification before reading.
    end_rx.recv_timeout(Duration::from_secs(60)).expect("timed out waiting for progress end notification");

    let notifications = progress_notifications.lock().unwrap().clone();

    assert!(notifications.len() >= 2, "expected at least begin+end progress notifications, got {}", notifications.len());

    // First notification should be a "begin".
    let begin = notifications[0].value.begin.as_ref().expect("expected first progress notification to be 'begin'");
    assert_eq!(begin.title, "Loading");

    // Last notification should be an "end".
    assert!(notifications.last().unwrap().value.end.is_some(), "expected last progress notification to be 'end'");

    // All notifications should share the same token.
    let first_token = token_string(&notifications[0].token);
    assert!(!first_token.is_empty(), "expected non-empty progress token");
    for (i, n) in notifications.iter().enumerate() {
        assert_eq!(token_string(&n.token), first_token, "notification {} has different token", i);
    }

    close_client.close().unwrap();
}

// server_progress_test.go:128
fn token_string(t: &lsproto::IntegerOrString) -> String {
    t.string.clone().unwrap_or_default()
}
