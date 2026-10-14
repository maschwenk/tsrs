use crate::*;

// lsp_test.go:10
#[test]
fn test_unmarshal_completion_item() {
    const MESSAGE: &str = r#"{
    "label": "pageXOffset",
    "insertTextFormat": 1,
    "textEdit": {
        "newText": "pageXOffset",
        "insert": {
            "start": {
                "line": 4,
                "character": 0
            },
            "end": {
                "line": 4,
                "character": 4
            }
        },
        "replace": {
            "start": {
                "line": 4,
                "character": 0
            },
            "end": {
                "line": 4,
                "character": 4
            }
        }
    },
    "kind": 6,
    "sortText": "15",
    "commitCharacters": [
        ".",
        ",",
        ";"
    ]
}"#;

    let result = unmarshal::<CompletionItem>(MESSAGE.as_bytes()).unwrap();

    assert_eq!(
        result,
        CompletionItem {
            label: "pageXOffset".to_string(),
            insert_text_format: Some(InsertTextFormat::PlainText),
            text_edit: Some(TextEditOrInsertReplaceEdit {
                insert_replace_edit: Some(InsertReplaceEdit {
                    new_text: "pageXOffset".to_string(),
                    insert: Range { start: Position { line: 4, character: 0 }, end: Position { line: 4, character: 4 } },
                    replace: Range { start: Position { line: 4, character: 0 }, end: Position { line: 4, character: 4 } },
                }),
                ..Default::default()
            }),
            kind: Some(CompletionItemKind::Variable),
            sort_text: Some("15".to_string()),
            commit_characters: Some(vec![".".to_string(), ",".to_string(), ";".to_string()]),
            ..Default::default()
        }
    );
}

// tsrs-only: `DocumentUri::check` rejects the URIs `file_name` cannot turn into a file name (Go panics on them, ending
// the session), and accepts those it can.
#[test]
fn check_rejects_uris_without_a_file_name() {
    for bad in ["", "not a uri", "%%%", "relative/path.ts", "file://", "file:///%zz", "1abc:x", "foo://host"] {
        assert!(DocumentUri::from(bad).check().is_err(), "{bad:?} passed");
    }
    for good in ["file:///a/b.ts", "file:///c%3A/a/b.ts", "file://server/share/a.ts", "untitled:Untitled-1", "vscode-notebook-cell:/a.ipynb#x", "foo://host/x", "bundled:///libs/lib.d.ts"] {
        let uri = DocumentUri::from(good);
        uri.check().unwrap_or_else(|err| panic!("{good:?}: {err}"));
        assert!(tsrs_core::tspath::path_is_absolute(&uri.file_name()) || !good.starts_with("file:"), "{good:?}");
    }
}
