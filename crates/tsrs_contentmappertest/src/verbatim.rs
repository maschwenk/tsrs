use tsrs_contentmapper::{ProtocolJson, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;

use crate::protocol::{identity_mapped_output, initialize_result, no_notifications, testHandler, unexpected_method, unmarshal_params};

// verbatim.go:11
pub(crate) struct verbatimHandler;

// verbatim.go:13
pub(crate) struct moduleVerbatimHandler;

impl testHandler for verbatimHandler {}

impl ipc::Handler for verbatimHandler {
    // verbatim.go:15
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                let mapped_output = identity_mapped_output(&p.content)?;
                Ok(TransformResult { mapped_output, ..Default::default() }.marshal_json())
            }
            _ => Err(unexpected_method(method)),
        }
    }

    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        no_notifications()
    }
}

impl testHandler for moduleVerbatimHandler {}

impl ipc::Handler for moduleVerbatimHandler {
    // verbatim.go:34
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                let mut mapped_output = identity_mapped_output(&p.content)?;
                mapped_output.extension = ".mts".to_string();
                Ok(TransformResult { mapped_output, ..Default::default() }.marshal_json())
            }
            _ => Err(unexpected_method(method)),
        }
    }

    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        no_notifications()
    }
}
