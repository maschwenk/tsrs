// Decoder strictness against a Go binary built at the pinned commit (tests/fixtures/go_json_oracle_main.go.txt,
// inputs from go_json_oracle_cases.py): duplicate member names, unpaired surrogate escapes and invalid
// UTF-8 are rejected with jsontext's error text; valid input (including escaped astral pairs) decodes.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use serde_json::Value;
use tsrs_api_transport::callbackfs::{CallbackConfig, CallbackFs};
use tsrs_api_transport::jsonrpc::decode_message;
use tsrs_api_transport::{Caller, TransportError};
use tsrs_vfs::FS;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

struct Fixed(Vec<u8>);

impl Caller for Fixed {
    fn call(&self, _method: &str, _params: Option<&[u8]>) -> Result<Vec<u8>, TransportError> {
        Ok(self.0.clone())
    }
    fn notify(&self, _: &str, _: Option<&[u8]>) -> Result<(), TransportError> {
        Ok(())
    }
}

fn read_file_via_callback(response: Vec<u8>) -> Result<Option<String>, String> {
    let fs = CallbackFs::new(Arc::new(tsrs_vfs::osvfs::fs()), &CallbackConfig::parse(&["readFile"]).unwrap(), None);
    fs.set_connection(Arc::new(Fixed(response)));
    catch_unwind(AssertUnwindSafe(|| fs.read_file("/x.ts"))).map_err(|p| p.downcast_ref::<String>().cloned().unwrap_or_default())
}

#[test]
fn decoders_match_pinned_go() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/go_json_oracle.json")).unwrap();
    let (mut rejected, mut accepted) = (0, 0);
    for case in &cases {
        let input = hex(case["input"].as_str().unwrap());
        let shown = String::from_utf8_lossy(&input).into_owned();
        let go_err = case["error"].as_str().unwrap();
        match case["kind"].as_str().unwrap() {
            "requestfs" => {
                #[cfg(feature = "requestfs")]
                {
                    let ours = tsrs_api_transport::requestfs::RequestFileSystemParams::from_json(&input);
                    if go_err.is_empty() {
                        assert!(ours.is_ok(), "{shown}: {ours:?}");
                    } else {
                        assert_eq!(ours.unwrap_err(), go_err, "{shown}");
                    }
                }
                #[cfg(not(feature = "requestfs"))]
                {
                    let ours = tsrs_api_transport::strictjson::validate(&input);
                    if go_err.is_empty() {
                        assert!(ours.is_ok(), "{shown}: {ours:?}");
                    } else {
                        assert_eq!(ours.unwrap_err(), go_err, "{shown}");
                    }
                }
            }
            "readFile" => {
                let ours = read_file_via_callback(input.clone());
                if go_err.is_empty() {
                    let content = ours.unwrap_or_else(|e| panic!("{shown}: {e}")).unwrap();
                    assert_eq!(content.as_bytes(), &hex(case["hex"].as_str().unwrap())[..], "{shown}");
                } else {
                    assert_eq!(ours.unwrap_err(), go_err.strip_prefix("response: ").unwrap(), "{shown}");
                }
            }
            "jsonrpc" => {
                let ours = decode_message(&input);
                if go_err.is_empty() {
                    assert!(ours.is_ok(), "{shown}: {ours:?}");
                } else {
                    assert_eq!(ours.unwrap_err().to_string(), go_err, "{shown}");
                }
            }
            other => panic!("unknown case kind {other}"),
        }
        if go_err.is_empty() {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }
    assert_eq!((accepted, rejected), (10, 28));
}

#[test]
fn deep_nesting_is_bounded() {
    let deep = "[".repeat(20000);
    let err = tsrs_api_transport::strictjson::validate(deep.as_bytes()).unwrap_err();
    assert!(err.contains("exceeded max depth"), "{err}");
    let ok = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    assert!(tsrs_api_transport::strictjson::validate(ok.as_bytes()).is_ok());
}

/// Grammar errors: every input of go_syntax_oracle.json (decoded by the pinned Go binary into `any`)
/// is accepted or rejected exactly as Go does, with Go's error text (pointer and offset rules included).
#[test]
fn syntax_errors_match_pinned_go_text() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/go_syntax_oracle.json")).unwrap();
    let mut mismatches = Vec::new();
    for case in &cases {
        let input = hex(case["input"].as_str().unwrap());
        let go = case["error"].as_str().unwrap();
        let ours = tsrs_api_transport::strictjson::validate(&input).err().unwrap_or_default();
        if ours != go {
            mismatches.push(format!("{:?}\n   go: {go}\n  ours: {ours}", String::from_utf8_lossy(&input)));
        }
    }
    assert!(cases.len() >= 100, "{}", cases.len());
    assert!(mismatches.is_empty(), "{} of {} differ:\n{}", mismatches.len(), cases.len(), mismatches.join("\n"));
}

/// readFile `value` decoding follows Go's json.Unmarshal into a string (absent, null, wrong kinds).
#[test]
fn read_file_value_kinds_match_pinned_go() {
    let cases = [
        (r#"{"kind":"value"}"#, Err("jsontext: unexpected EOF")),
        (r#"{"kind":"value","value":null}"#, Ok("")),
        (r#"{"kind":"value","value":5}"#, Err("json: cannot unmarshal JSON number into Go string")),
        (r#"{"kind":"value","value":true}"#, Err("json: cannot unmarshal JSON boolean into Go string")),
        (r#"{"kind":"value","value":[]}"#, Err("json: cannot unmarshal JSON array into Go string")),
        (r#"{"kind":"value","value":{}}"#, Err("json: cannot unmarshal JSON object into Go string")),
        (r#"{"kind":"value","value":-1.5e3}"#, Err("json: cannot unmarshal JSON number into Go string")),
    ];
    for (input, want) in cases {
        let got = read_file_via_callback(input.as_bytes().to_vec());
        match want {
            Ok(content) => assert_eq!(got.as_ref().map(|c| c.as_deref()), Ok(Some(content)), "{input}"),
            Err(text) => assert_eq!(got.unwrap_err(), text, "{input}"),
        }
    }
}
