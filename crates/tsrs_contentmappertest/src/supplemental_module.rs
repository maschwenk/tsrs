use tsrs_contentmapper::{MappedOutput, ProtocolJson, SupplementalOutput, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;

use crate::protocol::{initialize_result, no_notifications, testHandler, unexpected_method, unmarshal_params};

// supplemental_module.go:11
pub(crate) struct supplementalModuleHandler;

impl testHandler for supplementalModuleHandler {}

impl ipc::Handler for supplementalModuleHandler {
    // supplemental_module.go:13
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let _p: TransformParams = unmarshal_params(params)?;
                Ok(TransformResult {
                    mapped_output: MappedOutput { text: "export default 1;".to_string(), extension: ".ts".to_string(), ..Default::default() },
                    supplemental: vec![SupplementalOutput {
                        mapped_output: MappedOutput {
                            text: r#"export const privateValue: number = "wrong";"#.to_string(),
                            extension: ".ts".to_string(),
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
