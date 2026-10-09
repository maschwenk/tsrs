use tsrs_contentmapper::{ProtocolJson, SupplementalOutput, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;

use crate::protocol::{identity_mapped_output, initialize_result, no_notifications, testHandler, unexpected_method, unmarshal_params};

// duplicateProjectionHandler emits the original content as both the canonical output and a supplemental
// one, so a single original span has an identical counterpart in two distinct virtual source files that
// share an OriginalFileName. An edit to that span is therefore recorded once per projection.
// duplicate_projection.go:14
pub(crate) struct duplicateProjectionHandler;

impl testHandler for duplicateProjectionHandler {}

impl ipc::Handler for duplicateProjectionHandler {
    // duplicate_projection.go:16
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                let canonical = identity_mapped_output(&p.content)?;
                let supplemental = identity_mapped_output(&p.content)?;
                Ok(TransformResult {
                    mapped_output: canonical,
                    supplemental: vec![SupplementalOutput { mapped_output: supplemental }],
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
