mod common;
use common::*;
use tsrs_core::json::Value;

#[test]
fn batch_requests_results_errors_and_pagination() {
    let dir = TempDir::new("batch");
    let s = session(&dir.dir(), false);
    let r = call(&s, "batchRequests", r#"{"requests":[{"method":"initialize"},{"method":"nope","params":{}},{"method":"batchRequests","params":{"requests":[]}},{"method":"ping"}]}"#);
    assert_eq!(str_of(get(&r, "responses.0.method")), "initialize");
    assert_eq!(str_of(get(&r, "responses.0.result.currentDirectory")), dir.dir());
    assert!(str_of(get(&r, "responses.1.error")).contains("unknown API method"));
    assert_eq!(get(&r, "responses.1.result"), &Value::Null);
    assert_eq!(str_of(get(&r, "responses.2.error")), "api: invalid request: batchRequests cannot be nested");
    assert_eq!(str_of(get(&r, "responses.3.result")), "pong");
    assert!(matches!(r, Value::Object(ref o) if !o.contains_key("continuationToken")));

    // Tiny page size: one response per page, followed via continuation tokens.
    let r = call(&s, "batchRequests", r#"{"requests":[{"method":"ping"},{"method":"ping"},{"method":"ping"}],"maxResponseBytesPerPage":1}"#);
    let mut seen = match get(&r, "responses") { Value::Array(a) => a.len(), _ => 0 };
    assert_eq!(seen, 1);
    let mut token = str_of(get(&r, "continuationToken")).to_string();
    loop {
        let r = call(&s, "batchRequests", &format!("{{\"requests\":[],\"continuationToken\":{},\"maxResponseBytesPerPage\":1}}", quote(&token)));
        seen += match get(&r, "responses") { Value::Array(a) => a.len(), _ => 0 };
        match r {
            Value::Object(ref o) if o.contains_key("continuationToken") => token = str_of(get(&r, "continuationToken")).to_string(),
            _ => break,
        }
    }
    assert_eq!(seen, 3);
    let e = call_err(&s, "batchRequests", &format!("{{\"continuationToken\":{}}}", quote(&token)));
    assert!(e.contains("invalid batch continuation token"), "{e}");
}
