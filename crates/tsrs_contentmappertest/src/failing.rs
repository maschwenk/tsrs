use tsrs_contentmapper::{ProtocolJson, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;

use crate::protocol::{initialize_result, no_notifications, testHandler, unexpected_method};

// failing.go:12
pub(crate) struct failingHandler;

impl testHandler for failingHandler {}

impl ipc::Handler for failingHandler {
    // failing.go:14
    fn handle_request(&self, method: &str, _params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => Err(ipc::Error::new("content mapper failed to transform the file")),
            _ => Err(unexpected_method(method)),
        }
    }

    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        no_notifications()
    }
}
