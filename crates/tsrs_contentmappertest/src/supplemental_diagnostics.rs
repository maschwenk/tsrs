use tsrs_contentmapper::{MappedOutput, ProtocolJson, SupplementalOutput, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;
use tsrs_spanmap::{Feature, Kind, Segment};

use crate::protocol::{initialize_result, marshal_span_map, no_notifications, testHandler, unexpected_method, unmarshal_params};

// supplemental_diagnostics.go:13
pub(crate) struct supplementalDiagnosticsHandler;

impl testHandler for supplementalDiagnosticsHandler {}

impl ipc::Handler for supplementalDiagnosticsHandler {
    // supplemental_diagnostics.go:15
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                const PREFIX: &str = "missingSupplementalGlobal;\n";
                let mappings = marshal_span_map(&[Segment {
                    virtual_start: PREFIX.len() as i32,
                    virtual_end: (PREFIX.len() + p.content.len()) as i32,
                    original_end: p.content.len() as i32,
                    kind: Kind::Verbatim,
                    features: Feature::All,
                    ..Default::default()
                }])?;
                Ok(TransformResult {
                    mapped_output: MappedOutput { text: "export {};".to_string(), extension: ".ts".to_string(), ..Default::default() },
                    supplemental: vec![SupplementalOutput {
                        mapped_output: MappedOutput {
                            text: format!("{PREFIX}{}", p.content),
                            extension: ".ts".to_string(),
                            mappings: Some(mappings),
                            ..Default::default()
                        },
                    }],
                    ..Default::default()
                }
                .marshal_json())
            }
            _ => Err(unexpected_method(method)),
        }
    }

    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        no_notifications()
    }
}
