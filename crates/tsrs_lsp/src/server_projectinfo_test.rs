use std::sync::Arc;

use tsrs_lsproto as lsproto;

use crate::lsptestutil::{acknowledge_registrations, closeClient, new_lsp_client, test_server_options, LSPClient};

// server_projectinfo_test.go:16
fn expected_code_action_kinds() -> Vec<lsproto::CodeActionKind> {
    vec![
        lsproto::CodeActionKind::QuickFix,
        lsproto::CodeActionKind("source.organizeImports.ts"),
        lsproto::CodeActionKind("source.removeUnusedImports.ts"),
        lsproto::CodeActionKind("source.sortImports.ts"),
        lsproto::CodeActionKind("source.fixAll.ts"),
    ]
}

// server_projectinfo_test.go:26
fn init_project_info_client(files: &[(&str, &str)]) -> (Arc<LSPClient>, closeClient) {
    let (client, close_client) = new_lsp_client(test_server_options("/home/projects", files), Some(acknowledge_registrations()));

    let (init_msg, _) = client.send_request(lsproto::INITIALIZE_INFO, lsproto::InitializeParams::default());
    assert!(init_msg.error.is_none(), "Initialize failed");
    client.send_notification(lsproto::INITIALIZED_INFO, lsproto::InitializedParams::default());
    client.server.init_complete().wait();

    (client, close_client)
}

// server_projectinfo_test.go:61
#[test]
fn test_initialize_code_action_kinds() {
    let (client, close_client) = new_lsp_client(test_server_options("/home/projects", &[]), None);

    let (message, result) = client.send_request(lsproto::INITIALIZE_INFO, lsproto::InitializeParams::default());
    assert!(message.error.is_none(), "Initialize failed");
    let result = result.unwrap();
    assert_eq!(result.capabilities.code_action_provider.unwrap().code_action_options.unwrap().code_action_kinds.unwrap(), expected_code_action_kinds());
    let _ = close_client.close();
}

fn project_info_config_file_path(files: &[(&str, &str)]) -> String {
    let (client, close_client) = init_project_info_client(files);

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

    let (msg, resp) = client.send_request(lsproto::CUSTOM_PROJECT_INFO_INFO, lsproto::ProjectInfoParams { text_document: lsproto::TextDocumentIdentifier { uri } });
    assert!(msg.error.is_none(), "{:?}", msg.error);
    let _ = close_client.close();
    resp.expect("expected a response").config_file_path
}

// server_projectinfo_test.go:78
#[test]
fn test_project_info_configured_project() {
    if !tsrs_vfs::bundled::EMBEDDED {
        return;
    }
    let path = project_info_config_file_path(&[("/home/projects/tsconfig.json", "{}"), ("/home/projects/index.ts", "export const x = 1;")]);
    assert_eq!(path, "/home/projects/tsconfig.json");
}

// server_projectinfo_test.go:103
#[test]
fn test_project_info_inferred_project() {
    if !tsrs_vfs::bundled::EMBEDDED {
        return;
    }
    let path = project_info_config_file_path(&[("/home/projects/index.ts", "export const x = 1;")]);
    assert_eq!(path, "");
}
