use tsrs_contentmapper::{Diagnostic, ProtocolJson, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_diagnostics as diagnostics;
use tsrs_ipc as ipc;

use crate::protocol::{identity_mapped_output, initialize_result, no_notifications, testHandler, unexpected_method, unmarshal_params};

// diagnostic_code_collision.go:13
pub(crate) struct diagnosticCodeCollisionHandler;

impl testHandler for diagnosticCodeCollisionHandler {}

impl ipc::Handler for diagnosticCodeCollisionHandler {
    // diagnostic_code_collision.go:15
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                let mapped_output = identity_mapped_output(&p.content)?;
                // strings.Index: -1 when absent.
                let start = p.content.find("foo").map_or(-1, |start| start as i64);
                Ok(TransformResult {
                    mapped_output,
                    diagnostics: vec![Diagnostic {
                        message_text: "Mapper diagnostic with a colliding code.".to_string(),
                        start,
                        length: "foo".len() as i64,
                        code: diagnostics::Function_must_have_an_explicit_return_type_annotation_with_isolatedDeclarations.code(),
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
