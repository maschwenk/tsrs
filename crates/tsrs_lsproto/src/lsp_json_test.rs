use crate::json::{Json, JsonError, ObjectWriter, Value};
use crate::structcodec::*;
use crate::*;

fn unmarshal_str<T: Json>(input: &str) -> Result<T, JsonError> {
    unmarshal::<T>(input.as_bytes())
}

fn error_text<T: Json>(input: &str) -> String {
    match unmarshal_str::<T>(input) {
        Ok(_) => panic!("expected error for input {input}"),
        Err(err) => err.to_string(),
    }
}

// lsp_json_test.go:12
#[test]
fn test_unmarshal_rejects_null_for_optional_non_nullable_fields() {
    let tests: Vec<(&str, String, &str)> = vec![
        (
            "InlayHint kind null",
            error_text::<InlayHint>(r#"{"position": {"line": 0, "character": 0}, "label": "foo", "kind": null}"#),
            r#"null value is not allowed for field "kind""#,
        ),
        (
            "InlayHint textEdits null",
            error_text::<InlayHint>(r#"{"position": {"line": 0, "character": 0}, "label": "foo", "textEdits": null}"#),
            r#"null value is not allowed for field "textEdits""#,
        ),
        (
            "InlayHint paddingLeft null",
            error_text::<InlayHint>(r#"{"position": {"line": 0, "character": 0}, "label": "foo", "paddingLeft": null}"#),
            r#"null value is not allowed for field "paddingLeft""#,
        ),
        (
            "FoldingRange kind null",
            error_text::<FoldingRange>(r#"{"startLine": 0, "endLine": 10, "kind": null}"#),
            r#"null value is not allowed for field "kind""#,
        ),
        (
            "FoldingRange startCharacter null",
            error_text::<FoldingRange>(r#"{"startLine": 0, "endLine": 10, "startCharacter": null}"#),
            r#"null value is not allowed for field "startCharacter""#,
        ),
        (
            "CompletionItem insertTextFormat null",
            error_text::<CompletionItem>(r#"{"label": "test", "insertTextFormat": null}"#),
            r#"null value is not allowed for field "insertTextFormat""#,
        ),
        (
            "Hover range null",
            error_text::<Hover>(r#"{"contents": {"kind": "plaintext", "value": "hi"}, "range": null}"#),
            r#"null value is not allowed for field "range""#,
        ),
        (
            "WorkDoneProgressOptions workDoneProgress null",
            error_text::<WorkDoneProgressOptions>(r#"{"workDoneProgress": null}"#),
            r#"null value is not allowed for field "workDoneProgress""#,
        ),
        (
            "CallHierarchyIncomingCallsParams item null",
            error_text::<CallHierarchyIncomingCallsParams>(r#"{"item": null}"#),
            r#"null value is not allowed for field "item""#,
        ),
        (
            "CallHierarchyIncomingCall from null",
            error_text::<CallHierarchyIncomingCall>(r#"{"from": null, "fromRanges": []}"#),
            r#"null value is not allowed for field "from""#,
        ),
        (
            "InitializeParams capabilities null",
            error_text::<InitializeParams>(r#"{"processId": null, "rootUri": null, "capabilities": null}"#),
            r#"null value is not allowed for field "capabilities""#,
        ),
        (
            "InitializeResult capabilities null",
            error_text::<InitializeResult>(r#"{"capabilities": null}"#),
            r#"null value is not allowed for field "capabilities""#,
        ),
        (
            "SemanticTokens data null (required slice)",
            error_text::<SemanticTokens>(r#"{"data": null}"#),
            r#"null value is not allowed for field "data""#,
        ),
        (
            "TextDocumentEdit edits null (required slice)",
            error_text::<TextDocumentEdit>(r#"{"textDocument": {"uri": "file:///a.ts", "version": 1}, "edits": null}"#),
            r#"null value is not allowed for field "edits""#,
        ),
    ];

    for (name, err, err_text) in tests {
        assert!(err.contains(err_text), "{name}: {err}");
    }
}

// lsp_json_test.go:116
#[test]
fn test_unmarshal_accepts_null_for_nullable_fields() {
    unmarshal_str::<InitializeParams>(r#"{"processId": null, "rootUri": null, "capabilities": {}}"#).unwrap();
    unmarshal_str::<InitializeParams>(r#"{"processId": null, "rootUri": null, "capabilities": {}, "workspaceFolders": null}"#).unwrap();
    unmarshal_str::<InitializeParams>(r#"{"processId": null, "rootUri": null, "capabilities": {}}"#).unwrap();
    unmarshal_str::<InitializationOptions>(r#"{"userPreferences": null}"#).unwrap();
    unmarshal_str::<InitializeParams>(r#"{"processId": null, "rootUri": null, "capabilities": {}, "initializationOptions": null}"#)
        .unwrap();
}

// lsp_json_test.go:160
#[test]
fn test_unmarshal_accepts_omitted_optional_fields() {
    let hint = unmarshal_str::<InlayHint>(r#"{"position": {"line": 1, "character": 5}, "label": "test"}"#).unwrap();
    assert!(hint.kind.is_none());
    assert!(hint.text_edits.is_none());
    assert!(hint.tooltip.is_none());
    assert!(hint.padding_left.is_none());
    assert!(hint.padding_right.is_none());
    assert!(hint.data.is_none());
    assert_eq!(hint.position.line, 1);
    assert_eq!(hint.position.character, 5);

    let fr = unmarshal_str::<FoldingRange>(r#"{"startLine": 5, "endLine": 10}"#).unwrap();
    assert!(fr.kind.is_none());
    assert!(fr.start_character.is_none());
    assert!(fr.end_character.is_none());
    assert!(fr.collapsed_text.is_none());
    assert_eq!(fr.start_line, 5);
    assert_eq!(fr.end_line, 10);
}

// lsp_json_test.go:213
#[test]
fn test_unmarshal_rejects_incomplete_objects() {
    let tests = [
        ("InlayHint missing position", error_text::<InlayHint>(r#"{"label": "test"}"#), "missing required properties: position"),
        (
            "InlayHint missing label",
            error_text::<InlayHint>(r#"{"position": {"line": 0, "character": 0}}"#),
            "missing required properties: label",
        ),
        (
            "Location missing uri",
            error_text::<Location>(r#"{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}}"#),
            "missing required properties: uri",
        ),
        ("Location empty object", error_text::<Location>(r#"{}"#), "missing required properties: uri, range"),
    ];

    for (name, err, err_text) in tests {
        assert!(err.contains(err_text), "{name}: {err}");
    }
}

fn round_trip<T: Json + PartialEq + std::fmt::Debug>(value: &T) {
    let data = marshal(value).unwrap();
    let result = unmarshal::<T>(data.as_bytes()).unwrap();
    assert_eq!(*value, result);
}

// lsp_json_test.go:257
#[test]
fn test_marshal_unmarshal_round_trip() {
    round_trip(&InlayHint {
        position: Position { line: 1, character: 5 },
        label: StringOrInlayHintLabelParts { string: Some("param".to_string()), ..Default::default() },
        kind: Some(InlayHintKind::Parameter),
        ..Default::default()
    });
    round_trip(&InlayHint {
        position: Position { line: 0, character: 0 },
        label: StringOrInlayHintLabelParts { string: Some("x".to_string()), ..Default::default() },
        ..Default::default()
    });
    round_trip(&FoldingRange {
        start_line: 1,
        start_character: Some(0),
        end_line: 10,
        end_character: Some(5),
        kind: Some(FoldingRangeKind::Region),
        collapsed_text: Some("...".to_string()),
    });
    round_trip(&Location {
        uri: DocumentUri::from("file:///test.ts"),
        range: Range { start: Position { line: 1, character: 2 }, end: Position { line: 3, character: 4 } },
    });
    round_trip(&InitializeParams {
        process_id: IntegerOrNull::default(),
        root_uri: DocumentUriOrNull { document_uri: Some(DocumentUri::from("file:///workspace")) },
        capabilities: ClientCapabilities::default(),
        ..Default::default()
    });
}

// lsp_json_test.go:345
#[test]
fn test_unmarshal_union_types() {
    let v = unmarshal_str::<IntegerOrString>("42").unwrap();
    assert_eq!(v.integer, Some(42));
    assert!(v.string.is_none());

    let v = unmarshal_str::<IntegerOrString>(r#""hello""#).unwrap();
    assert_eq!(v.string.as_deref(), Some("hello"));
    assert!(v.integer.is_none());

    let v = unmarshal_str::<IntegerOrNull>("42").unwrap();
    assert_eq!(v.integer, Some(42));

    let v = unmarshal_str::<IntegerOrNull>("null").unwrap();
    assert!(v.integer.is_none());

    let v = unmarshal_str::<DocumentUriOrNull>(r#""file:///test.ts""#).unwrap();
    assert_eq!(v.document_uri, Some(DocumentUri::from("file:///test.ts")));

    let v = unmarshal_str::<DocumentUriOrNull>("null").unwrap();
    assert!(v.document_uri.is_none());
}

// lsp_json_test.go:403
#[test]
fn test_marshal_union_types() {
    assert_eq!(marshal(&IntegerOrNull { integer: Some(42) }).unwrap(), "42");
    assert_eq!(marshal(&IntegerOrNull::default()).unwrap(), "null");
    assert_eq!(marshal(&IntegerOrString { integer: Some(7), ..Default::default() }).unwrap(), "7");
    assert_eq!(marshal(&IntegerOrString { string: Some("tok".to_string()), ..Default::default() }).unwrap(), r#""tok""#);
}

// lsp_json_test.go:439
#[test]
fn test_unmarshal_ignores_unknown_fields() {
    let loc = unmarshal_str::<Location>(
        r#"{
			"uri": "file:///test.ts",
			"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 5}},
			"someUnknownField": 42,
			"anotherUnknown": {"nested": true}
		}"#,
    )
    .unwrap();
    assert_eq!(loc.uri, DocumentUri::from("file:///test.ts"));

    unmarshal_str::<InlayHint>(
        r#"{
			"position": {"line": 0, "character": 0},
			"label": "x",
			"futureField": [1, 2, 3]
		}"#,
    )
    .unwrap();
}

// lsp_json_test.go:467
#[test]
fn test_unmarshal_rejects_wrong_types() {
    assert!(unmarshal_str::<Location>("[]").is_err());
    assert!(unmarshal_str::<Location>(r#""not an object""#).is_err());
    assert!(unmarshal_str::<Location>("42").is_err());
    assert!(unmarshal_str::<Location>("null").is_err());
    assert!(unmarshal_str::<FoldingRange>("true").is_err());
}

// lsp_json_test.go:511
#[test]
fn test_unmarshal_union_type_wrong_kind() {
    assert!(unmarshal_str::<IntegerOrString>("true").is_err());
    assert!(unmarshal_str::<IntegerOrString>("null").is_err());
    assert!(unmarshal_str::<IntegerOrString>("{}").is_err());
    assert!(unmarshal_str::<IntegerOrString>("[]").is_err());
    assert!(unmarshal_str::<StringOrInlayHintLabelParts>("42").is_err());
    assert!(unmarshal_str::<StringOrInlayHintLabelParts>("true").is_err());
}

// lsp_json_test.go:557
#[test]
fn test_unmarshal_boolean_union_types() {
    let v = unmarshal_str::<BooleanOrHoverOptions>("true").unwrap();
    assert_eq!(v.boolean, Some(true));
    assert!(v.hover_options.is_none());

    let v = unmarshal_str::<BooleanOrHoverOptions>("false").unwrap();
    assert_eq!(v.boolean, Some(false));
    assert!(v.hover_options.is_none());

    let v = unmarshal_str::<BooleanOrHoverOptions>("{}").unwrap();
    assert!(v.boolean.is_none());
    assert!(v.hover_options.is_some());

    assert!(unmarshal_str::<BooleanOrHoverOptions>(r#""nope""#).is_err());
}

// lsp_json_test.go:597
#[derive(Default)]
struct OptionalDiscriminatorArm {
    kind: Option<StringLiteralBegin>,
    title: String,
}

impl Json for OptionalDiscriminatorArm {
    const GO_TYPE: &'static str = "lsproto.optionalDiscriminatorArm";

    fn to_json(&self) -> Value {
        let mut w = ObjectWriter::new(2);
        w.opt("kind", &self.kind);
        w.field("title", &self.title);
        w.finish()
    }

    fn from_json(v: &Value) -> Result<Self, JsonError> {
        let mut s = Self::default();
        let Some(members) = struct_members(v, Self::GO_TYPE, true)? else {
            return Ok(s);
        };
        let mut seen = 0u64;
        for (k, v) in members {
            match k.as_str() {
                "kind" => s.kind = opt_non_null(k, v, Self::GO_TYPE)?,
                "title" => {
                    seen |= 1;
                    s.title = field(k, v)?;
                }
                _ => {}
            }
        }
        check_required(seen, &["title"], Self::GO_TYPE)?;
        Ok(s)
    }
}

// lsp_json_test.go:602
#[test]
fn test_unmarshal_discriminator_union() {
    // WorkDoneProgressBegin
    let v = unmarshal_str::<WorkDoneProgressBeginOrReportOrEnd>(r#"{"kind": "begin", "title": "Indexing"}"#).unwrap();
    assert!(v.begin.is_some() && v.report.is_none() && v.end.is_none());
    assert_eq!(v.begin.unwrap().title, "Indexing");

    // WorkDoneProgressReport
    let v = unmarshal_str::<WorkDoneProgressBeginOrReportOrEnd>(r#"{"kind": "report", "message": "50%"}"#).unwrap();
    assert!(v.begin.is_none() && v.end.is_none());
    assert_eq!(v.report.unwrap().message.as_deref(), Some("50%"));

    // WorkDoneProgressEnd
    let v = unmarshal_str::<WorkDoneProgressBeginOrReportOrEnd>(r#"{"kind": "end"}"#).unwrap();
    assert!(v.begin.is_none() && v.report.is_none() && v.end.is_some());

    // discriminator after variant fields
    let v = unmarshal_str::<WorkDoneProgressBeginOrReportOrEnd>(r#"{"title": "Indexing", "percentage": 25, "kind": "begin"}"#).unwrap();
    let begin = v.begin.unwrap();
    assert_eq!(begin.title, "Indexing");
    assert_eq!(begin.percentage, Some(25));

    // optional discriminator is preserved
    let input = tsrs_core::json::unmarshal(r#"{"kind": "begin", "title": "Indexing"}"#).unwrap();
    let state = scan_discriminated_struct(&input, "lsproto.optionalDiscriminatorArm", "optionalDiscriminatorArm", "kind").unwrap();
    assert_eq!(state.discriminator_str(), Some("begin"));
    let v = unmarshal_discriminated_arm::<OptionalDiscriminatorArm>(&input, "lsproto.optionalDiscriminatorArm", "kind").unwrap();
    assert!(v.kind.is_some());
    assert_eq!(v.title, "Indexing");

    // invalid discriminator
    assert!(unmarshal_str::<WorkDoneProgressBeginOrReportOrEnd>(r#"{"kind": "invalid"}"#).is_err());

    // non-string discriminator
    assert!(unmarshal_str::<WorkDoneProgressBeginOrReportOrEnd>(r#"{"kind": null}"#).is_err());

    // missing discriminator
    let err = error_text::<WorkDoneProgressBeginOrReportOrEnd>(r#"{"message": "missing kind"}"#);
    assert!(err.contains(r#"missing discriminator "kind""#), "{err}");
}

// lsp_json_test.go:683
#[test]
fn test_unmarshal_presence_discriminator_union() {
    let v = unmarshal_str::<TextEditOrInsertReplaceEdit>(
        r#"{
			"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
			"newText": "x"
		}"#,
    )
    .unwrap();
    assert!(v.insert_replace_edit.is_none());
    assert_eq!(v.text_edit.unwrap().new_text, "x");

    let v = unmarshal_str::<TextEditOrInsertReplaceEdit>(
        r#"{
			"insert": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
			"replace": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}},
			"newText": "y"
		}"#,
    )
    .unwrap();
    assert!(v.text_edit.is_none());
    assert_eq!(v.insert_replace_edit.unwrap().new_text, "y");
}

// lsp_json_test.go:714
#[test]
fn test_unmarshal_string_or_array_union() {
    let v = unmarshal_str::<StringOrInlayHintLabelParts>(r#""hello""#).unwrap();
    assert_eq!(v.string.as_deref(), Some("hello"));
    assert!(v.inlay_hint_label_parts.is_none());

    let v = unmarshal_str::<StringOrInlayHintLabelParts>(r#"[{"value": "param"}, {"value": ": "}, {"value": "string"}]"#).unwrap();
    assert!(v.string.is_none());
    let parts = v.inlay_hint_label_parts.unwrap();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].value, "param");
}

// lsp_json_test.go:739
#[test]
fn test_unmarshal_document_edit_union() {
    type U = TextDocumentEditOrCreateFileOrRenameFileOrDeleteFile;

    // TextDocumentEdit without kind
    let v = unmarshal_str::<U>(
        r#"{
			"textDocument": {"uri": "file:///a.ts", "version": 1},
			"edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "x"}]
		}"#,
    )
    .unwrap();
    assert!(v.text_document_edit.is_some() && v.create_file.is_none() && v.rename_file.is_none() && v.delete_file.is_none());

    // TextDocumentEdit with non-string kind
    let v = unmarshal_str::<U>(
        r#"{
			"kind": null,
			"textDocument": {"uri": "file:///a.ts", "version": 1},
			"edits": []
		}"#,
    )
    .unwrap();
    assert!(v.text_document_edit.is_some() && v.create_file.is_none() && v.rename_file.is_none() && v.delete_file.is_none());

    // CreateFile with kind create
    let v = unmarshal_str::<U>(r#"{"kind": "create", "uri": "file:///new.ts"}"#).unwrap();
    assert!(v.text_document_edit.is_none());
    assert_eq!(v.create_file.unwrap().uri, DocumentUri::from("file:///new.ts"));

    // CreateFile with kind after fields
    let v = unmarshal_str::<U>(r#"{"uri": "file:///new.ts", "kind": "create"}"#).unwrap();
    assert_eq!(v.create_file.unwrap().uri, DocumentUri::from("file:///new.ts"));

    // RenameFile with kind rename
    let v = unmarshal_str::<U>(r#"{"kind": "rename", "oldUri": "file:///old.ts", "newUri": "file:///new.ts"}"#).unwrap();
    assert_eq!(v.rename_file.unwrap().old_uri, DocumentUri::from("file:///old.ts"));

    // DeleteFile with kind delete
    let v = unmarshal_str::<U>(r#"{"kind": "delete", "uri": "file:///gone.ts"}"#).unwrap();
    assert_eq!(v.delete_file.unwrap().uri, DocumentUri::from("file:///gone.ts"));
}

// lsp_json_test.go:809
#[test]
fn test_unmarshal_field_ordering() {
    let loc = unmarshal_str::<Location>(
        r#"{
			"range": {"start": {"line": 1, "character": 2}, "end": {"line": 3, "character": 4}},
			"uri": "file:///test.ts"
		}"#,
    )
    .unwrap();
    assert_eq!(loc.uri, DocumentUri::from("file:///test.ts"));
    assert_eq!(loc.range.start.line, 1);

    let hint = unmarshal_str::<InlayHint>(
        r#"{
			"kind": 1,
			"label": "x",
			"position": {"line": 0, "character": 0}
		}"#,
    )
    .unwrap();
    assert_eq!(hint.kind, Some(InlayHintKind::Type));
}

// lsp_json_test.go:838
#[test]
fn test_unmarshal_empty_object() {
    let v = unmarshal_str::<WorkDoneProgressOptions>("{}").unwrap();
    assert!(v.work_done_progress.is_none());
    unmarshal_str::<InitializationOptions>("{}").unwrap();
    unmarshal_str::<ClientCapabilities>("{}").unwrap();
    unmarshal_str::<ServerCapabilities>("{}").unwrap();
}

// lsp_json_test.go:871
#[test]
fn test_marshal_omits_zero_optional_fields() {
    let hint = InlayHint {
        position: Position { line: 0, character: 0 },
        label: StringOrInlayHintLabelParts { string: Some("x".to_string()), ..Default::default() },
        ..Default::default()
    };
    let s = marshal(&hint).unwrap();
    assert!(!s.contains("kind"), "should not contain 'kind', got: {s}");
    assert!(!s.contains("textEdits"), "should not contain 'textEdits', got: {s}");
    assert!(!s.contains("paddingLeft"), "should not contain 'paddingLeft', got: {s}");
    assert!(s.contains("position"), "should contain 'position', got: {s}");
    assert!(s.contains("label"), "should contain 'label', got: {s}");

    let fr = FoldingRange { start_line: 1, end_line: 10, ..Default::default() };
    let s = marshal(&fr).unwrap();
    assert!(!s.contains("kind"), "should not contain 'kind', got: {s}");
    assert!(!s.contains("startCharacter"), "should not contain 'startCharacter', got: {s}");
    assert!(s.contains("startLine"), "should contain 'startLine', got: {s}");
    assert!(s.contains("endLine"), "should contain 'endLine', got: {s}");
}

// lsp_json_test.go:903
#[test]
fn test_literal_types() {
    assert_eq!(marshal(&StringLiteralCreate).unwrap(), r#""create""#);
    unmarshal_str::<StringLiteralCreate>(r#""create""#).unwrap();
    assert!(unmarshal_str::<StringLiteralCreate>(r#""delete""#).is_err());
    assert!(unmarshal_str::<StringLiteralCreate>("42").is_err());
}

// lsp_json_test.go:936
#[test]
fn test_enum_string_values() {
    assert_eq!(InlayHintKind::Type.string(), "Type");
    assert_eq!(InlayHintKind::Parameter.string(), "Parameter");

    assert_eq!(SymbolKind::File.string(), "File");
    assert_eq!(SymbolKind::Function.string(), "Function");
    assert_eq!(SymbolKind::Variable.string(), "Variable");

    let s = InlayHintKind(999).string();
    assert!(s.contains("999"), "should contain the numeric value, got: {s}");
}

fn round_trip_marshal<T: Json>(value: &T) {
    let data = marshal(value).unwrap();
    let got = unmarshal::<T>(data.as_bytes()).unwrap();
    let again = marshal(&got).unwrap();
    assert_eq!(data, again, "re-marshal differs");
}

// lsp_json_test.go:966
// TestRoundTrip locks the generated codecs: every value must survive
// marshal -> unmarshal unchanged.
#[test]
fn test_round_trip() {
    round_trip_marshal(&Range { start: Position { line: 1, character: 2 }, end: Position { line: 3, character: 4 } });
    round_trip_marshal(&TextEdit {
        range: Range { start: Position { line: 1, character: 2 }, end: Position { line: 3, character: 4 } },
        new_text: "hello".to_string(),
    });
    round_trip_marshal(&MarkupContent { kind: MarkupKind::Markdown, value: "**x**".to_string() });
    round_trip_marshal(&DidChangeConfigurationParams {
        settings: tsrs_core::json::unmarshal(r#"{"js/ts":{"x":1}}"#).unwrap(),
    });
    round_trip_marshal(&DidChangeConfigurationParams { settings: Value::Null });
    round_trip_marshal(&CompletionItem {
        label: "pageXOffset".to_string(),
        kind: Some(CompletionItemKind::Field),
        sort_text: Some("15".to_string()),
        insert_text_format: Some(InsertTextFormat::PlainText),
        ..Default::default()
    });
    // StringOrTuple union (string arm and tuple arm).
    round_trip_marshal(&ParameterInformation {
        label: StringOrTuple { string: Some("p: number".to_string()), ..Default::default() },
        ..Default::default()
    });
    round_trip_marshal(&ParameterInformation {
        label: StringOrTuple { tuple: Some([0, 4]), ..Default::default() },
        ..Default::default()
    });
}

// lsp_json_test.go:1017
// TestStrictnessMissingRequired confirms required fields are still enforced.
#[test]
fn test_strictness_missing_required() {
    let errs = [
        error_text::<TextEdit>(r#"{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}"#),
        error_text::<Range>(r#"{"start":{"line":0,"character":0}}"#),
        error_text::<Position>(r#"{"line":0}"#),
    ];
    for err in errs {
        assert!(err.contains("missing required properties"), "{err}");
    }
}

// lsp_json_test.go:1039
// TestStrictnessNotObject confirms a non-object where an object is required
// is rejected rather than coerced.
#[test]
fn test_strictness_not_object() {
    let err = error_text::<TextEdit>(r#""oops""#);
    assert!(err.contains("object") || err.contains("cannot unmarshal"), "{err}");
}

// lsp_json_test.go:1049
// TestUnmarshalParamsRequiresParams verifies that a NoParams method must be
// given no params while every other method must be given params, and that a
// mismatch (including a null value either way) is an InvalidParams error.
#[test]
fn test_unmarshal_params_requires_params() {
    fn params(raw: Option<&str>) -> RequestMessage {
        RequestMessage { params: raw.map(|r| tsrs_core::json::unmarshal(r).unwrap()), ..Default::default() }
    }

    // NoParams: only truly-absent/empty params are accepted; null and any
    // present value are rejected. (An empty raw value is represented as absent.)
    let no_params_tests = [("absent", None, false), ("null", Some("null"), true), ("object", Some("{}"), true)];
    for (name, raw, want_err) in no_params_tests {
        let result = params(raw).unmarshal_params::<NoParams>();
        if want_err {
            assert!(result.unwrap_err().is_code(ErrorCode::InvalidParams), "NoParams/{name}");
        } else {
            result.unwrap();
        }
    }

    // Required-params method: only an object or array is accepted; absent,
    // empty, null, and other scalars are rejected.
    let typed_tests = [
        ("absent", None, true),
        ("null", Some("null"), true),
        ("number", Some("5"), true),
        ("string", Some(r#""x""#), true),
        ("object", Some(r#"{"settings":{"x":1}}"#), false),
    ];
    for (name, raw, want_err) in typed_tests {
        let result = params(raw).unmarshal_params::<DidChangeConfigurationParams>();
        if want_err {
            assert!(result.unwrap_err().is_code(ErrorCode::InvalidParams), "typed/{name}");
        } else {
            assert!(!matches!(result.unwrap().settings, Value::Null), "typed/{name}");
        }
    }
}
