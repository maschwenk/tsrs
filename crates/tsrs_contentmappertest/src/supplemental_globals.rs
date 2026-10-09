use tsrs_contentmapper::{MappedOutput, ProtocolJson, SupplementalOutput, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;

use crate::protocol::{initialize_result, no_notifications, quote, testHandler, unexpected_method, unmarshal_params};

// supplemental_globals.go:12
pub(crate) struct supplementalGlobalsHandler;

impl testHandler for supplementalGlobalsHandler {}

impl ipc::Handler for supplementalGlobalsHandler {
    // supplemental_globals.go:14
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                let supplemental = if p.file_name.ends_with("/a.vue") {
                    "/// <reference path=\"./extra.d.ts\" />\ninterface Shared extends Extra { value: string }"
                } else if p.file_name.ends_with("/b.vue") {
                    "declare const shared: Shared;"
                } else {
                    return Err(ipc::Error::new(format!(
                        "contentmappertest: unexpected supplemental global input {}",
                        quote(&p.file_name)
                    )));
                };
                Ok(TransformResult {
                    mapped_output: MappedOutput {
                        text: "export default shared.value;".to_string(),
                        extension: ".ts".to_string(),
                        ..Default::default()
                    },
                    supplemental: vec![SupplementalOutput {
                        mapped_output: MappedOutput { text: supplemental.to_string(), extension: ".ts".to_string(), ..Default::default() },
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
