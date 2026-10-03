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

/// Batch items keep their raw params bytes (Go `json.Value`): number literals are decoded as written, not
/// re-encoded from f64 (runtime af8 review).
#[test]
fn batch_items_decode_raw_params() {
    let dir = TempDir::new("batchraw");
    let s = session(&dir.dir(), false);
    let r = call(
        &s,
        "batchRequests",
        r#"{"requests":[
            {"method":"release","params":{"snapshot":1e3}},
            {"method":"release","params":{"snapshot":1.0}},
            {"method":"release","params":{"snapshot":9007199254740993}},
            {"method":"release","params":{"snapshot":18446744073709551615}},
            {"method":"release","params":{"snap\u0073hot":1e3}},
            {"par\u0061ms":{"snapshot":9007199254740995},"method":"release"}
        ]}"#,
    );
    for i in [0, 1, 4] {
        let e = str_of(get(&r, &format!("responses.{i}.error"))).to_string();
        assert!(e.starts_with("api: invalid request: failed to unmarshal *api.ReleaseParams"), "{i}: {e}");
    }
    assert_eq!(str_of(get(&r, "responses.2.error")), "api: client error: snapshot 9007199254740993 not found");
    assert_eq!(str_of(get(&r, "responses.3.error")), "api: client error: snapshot 18446744073709551615 not found");
    assert_eq!(str_of(get(&r, "responses.5.error")), "api: client error: snapshot 9007199254740995 not found");
}
