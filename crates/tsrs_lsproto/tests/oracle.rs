// Differential test against Go's lsproto codecs: every line of testdata/oracle.txt is
// "Type<TAB>input JSON<TAB>Go result", where the Go result is `json.Unmarshal` into a new value of the type
// followed by `json.Marshal` ("OK <json>"), or the unmarshal error ("ERR <message>").
// Regenerate with tools/oracle/lsproto/run.sh.

mod oracle_registry;

// json/v2 reports top-level errors with a byte offset into the input ("after offset N"); decoding from a
// parsed value has no offsets, so the offset is not compared. json/v2 also picks "cannot" or "unable to"
// at random per process; the port always says "cannot".
fn strip_offset(s: &str) -> String {
    let mut out = s.replace("json: unable to ", "json: cannot ");
    while let Some(i) = out.find(" after offset ") {
        let rest = &out[i + " after offset ".len()..];
        let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
        out = format!("{}{}", &out[..i], &rest[digits..]);
    }
    out
}

#[test]
fn oracle() {
    let data = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/oracle.txt")).unwrap();
    let mut failures = Vec::new();
    let mut count = 0;
    for (i, line) in data.lines().enumerate() {
        let mut parts = line.splitn(3, '\t');
        let (name, input, expected) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
        let got = oracle_registry::roundtrip(name, input.as_bytes()).unwrap_or_else(|| panic!("unknown type {name}"));
        count += 1;
        if strip_offset(&got) != strip_offset(expected) {
            failures.push(format!("line {}: {} {}\n  go:   {}\n  rust: {}", i + 1, name, input, expected, got));
        }
    }
    for f in failures.iter().take(40) {
        eprintln!("{f}");
    }
    assert!(failures.is_empty(), "{} of {} oracle cases differ", failures.len(), count);
}

// testdata/filename.txt: "uri<TAB>Go FileName()" (or "PANIC <message>").
#[test]
fn oracle_file_name() {
    let data = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/filename.txt")).unwrap();
    let mut failures = Vec::new();
    for line in data.lines() {
        let (uri, expected) = line.split_once('\t').unwrap();
        let uri = tsrs_lsproto::DocumentUri::from(uri);
        let got = match std::panic::catch_unwind(|| uri.file_name()) {
            Ok(name) => name,
            Err(payload) => format!("PANIC {}", payload.downcast_ref::<String>().cloned().unwrap_or_default()),
        };
        if got != expected {
            failures.push(format!("{uri}: go {expected:?}, rust {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// Outbound messages, against what Go's json.Marshal writes for the same values.
#[test]
fn oracle_messages() {
    use tsrs_lsproto::jsonrpc;
    use tsrs_lsproto::*;

    let req = RequestInfo::<NoParams, Null>::new(Method("x")).new_request_message(Some(jsonrpc::new_id_int(1)), NoParams);
    assert_eq!(marshal(&req.message()).unwrap(), r#"{"jsonrpc":"2.0","id":1,"method":"x","params":{}}"#);

    let resp = ResponseMessage { id: Some(jsonrpc::new_id_int(1)), result: Some(Null.to_json()), ..Default::default() };
    assert_eq!(marshal(&resp).unwrap(), r#"{"jsonrpc":"2.0","id":1,"result":null}"#);

    let resp = ResponseMessage {
        error: Some(jsonrpc::ResponseError { code: 1, message: "m".to_string(), data: None }),
        ..Default::default()
    };
    assert_eq!(marshal(&resp).unwrap(), r#"{"jsonrpc":"2.0","id":null,"error":{"code":1,"message":"m"}}"#);

    let resp = ResponseMessage { id: Some(jsonrpc::new_id_string("a")), result: Some(HoverOrNull::default().to_json()), ..Default::default() };
    assert_eq!(marshal(&resp.message()).unwrap(), r#"{"jsonrpc":"2.0","id":"a","result":null}"#);

    let note = TEXT_DOCUMENT_PUBLISH_DIAGNOSTICS_INFO.new_notification_message(PublishDiagnosticsParams {
        uri: DocumentUri::from("file:///a.ts"),
        version: Some(3),
        diagnostics: Vec::new(),
    });
    assert_eq!(
        marshal(&note).unwrap(),
        r#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file:///a.ts","version":3,"diagnostics":[]}}"#
    );
}
