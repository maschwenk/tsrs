use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::Arc;

use tsrs_contentmapper::{
    OpenProjectParams, OpenProjectResult, OptionDiagnosticResult, ProtocolJson, TransformParams, METHOD_CLOSE_PROJECT,
    METHOD_OPEN_PROJECT, METHOD_TRANSFORM,
};
use tsrs_core::json::Value;
use tsrs_core::tspath;
use tsrs_ipc as ipc;

use crate::protocol::{raw_text, testHandler, unmarshal_params};
use crate::verbatim::verbatimHandler;

// ProjectLifecycle records mapper project protocol calls.
// dynamic_verbatim.go:14
#[derive(Debug, Default)]
pub struct ProjectLifecycle {
    pub opens: AtomicI32,
    pub closes: AtomicI32,
}

// dynamic_verbatim.go:19 (Go embeds verbatimHandler, which carries no state.)
pub(crate) struct dynamicVerbatimHandler {
    pub(crate) lifecycle: Option<Arc<ProjectLifecycle>>,
}

impl testHandler for dynamicVerbatimHandler {}

impl ipc::Handler for dynamicVerbatimHandler {
    // dynamic_verbatim.go:24
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_OPEN_PROJECT => {
                if let Some(lifecycle) = &self.lifecycle {
                    lifecycle.opens.fetch_add(1, SeqCst);
                }
                let p: OpenProjectParams = unmarshal_params(params)?;
                let options = raw_text(p.options.as_ref());
                let identity = format!("{}:{}", p.config_file_name, options);
                let mut diagnostics = Vec::new();
                if options == r#"{"plugins":[{"name":1}]}"# {
                    diagnostics = vec![OptionDiagnosticResult {
                        path: vec![Value::String("plugins".to_string()), Value::Number(0.0), Value::String("name".to_string())],
                        message_text: "Option 'name' requires a string.".to_string(),
                        code: 123,
                    }];
                }
                let mut watch_directory = tspath::get_directory_path(&p.config_file_name);
                if watch_directory.is_empty() {
                    watch_directory = "/".to_string();
                }
                return Ok(OpenProjectResult {
                    config_identity: identity,
                    watched_files: vec![tspath::combine_paths(&watch_directory, &["mapper.config.json"])],
                    option_diagnostics: diagnostics,
                }
                .marshal_json());
            }
            METHOD_CLOSE_PROJECT => {
                if let Some(lifecycle) = &self.lifecycle {
                    lifecycle.closes.fetch_add(1, SeqCst);
                }
                return Ok(Value::Null);
            }
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                if p.project_handle.is_empty() {
                    return Err(ipc::Error::new("content mapper transform requires a project handle"));
                }
            }
            _ => {}
        }
        verbatimHandler.handle_request(method, params)
    }

    fn handle_notification(&self, method: &str, params: Option<&Value>) -> Result<(), ipc::Error> {
        verbatimHandler.handle_notification(method, params)
    }
}
