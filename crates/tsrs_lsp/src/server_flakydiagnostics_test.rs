use tsrs_lsproto as lsproto;

use crate::lsptestutil::{acknowledge_registrations, new_lsp_client, test_server_options};

// server_flakydiagnostics_test.go:17. (Go emits between the two diagnostics requests; this server's handler does not.)
#[test]
fn test_flaky_diagnostic_tracking_parallel_emit() {
    if !tsrs_vfs::bundled::EMBEDDED {
        return;
    }

    for no_emit_on_error in [false, true] {
        let tsconfig = format!(
            r#"{{
					"compilerOptions": {{ "strict": true, "declaration": true, "noEmitOnError": {}, "outDir": "out" }}
				}}"#,
            no_emit_on_error
        );
        let a = "export function box<T>(value: T) { return { value }; }";
        let files = [
            ("/src/tsconfig.json", tsconfig.as_str()),
            ("/src/a.ts", a),
            ("/src/b.ts", r#"import { box } from "./a"; export const b = box("b");"#),
            ("/src/c.ts", r#"import { box } from "./a"; export const c = box(1);"#),
        ];
        let (client, close_client) = new_lsp_client(test_server_options("/src", &files), Some(acknowledge_registrations()));

        let (msg, _) = client.send_request(
            lsproto::INITIALIZE_INFO,
            lsproto::InitializeParams {
                initialization_options: Some(lsproto::InitializationOptionsOrNull {
                    initialization_options: Some(lsproto::InitializationOptions {
                        track_flaky_diagnostics: Some(lsproto::DiagnosticFlakeLogLevel::Panic),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        assert!(msg.error.is_none(), "initialize failed");
        client.send_notification(lsproto::INITIALIZED_INFO, lsproto::InitializedParams::default());
        client.server.init_complete().wait();

        let uri = lsproto::DocumentUri::from("file:///src/a.ts");
        client.send_notification(
            lsproto::TEXT_DOCUMENT_DID_OPEN_INFO,
            lsproto::DidOpenTextDocumentParams {
                text_document: lsproto::TextDocumentItem { uri: uri.clone(), language_id: lsproto::LanguageKind::TypeScript, text: a.to_string(), ..Default::default() },
            },
        );
        let (msg, diagnostics) =
            client.send_request(lsproto::TEXT_DOCUMENT_DIAGNOSTIC_INFO, lsproto::DocumentDiagnosticParams { text_document: lsproto::TextDocumentIdentifier { uri }, ..Default::default() });
        assert!(msg.error.is_none(), "diagnostics request failed: {:?}", msg.error);
        let diagnostics = diagnostics.unwrap();
        let report = diagnostics.full_document_diagnostic_report.expect("full report");
        assert_eq!(report.items.len(), 0);
        let _ = close_client.close();
    }
}
