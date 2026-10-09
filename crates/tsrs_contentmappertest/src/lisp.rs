use tsrs_contentmapper::{MappedOutput, ProtocolJson, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;
use tsrs_spanmap::{Feature, Kind, Segment};

use crate::protocol::{initialize_result, marshal_span_map, no_notifications, quote, testHandler, unexpected_method, unmarshal_params};

// lisp.go:13
pub(crate) struct lispHandler;

impl testHandler for lispHandler {}

impl ipc::Handler for lispHandler {
    // lisp.go:15
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("lisp").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                if p.content.strip_suffix('\n').unwrap_or(&p.content) != r#"(+ 1 2 "oops")"# {
                    return Err(ipc::Error::new(format!("contentmappertest: unsupported Lisp expression {}", quote(&p.content))));
                }
                let segment = |virtual_start, virtual_end, original_start, original_end, kind| Segment {
                    virtual_start,
                    virtual_end,
                    original_start,
                    original_end,
                    kind,
                    features: Feature::All,
                };
                let mappings = marshal_span_map(&[
                    segment(0, 3, 1, 2, Kind::Alias),
                    segment(4, 5, 3, 4, Kind::Verbatim),
                    segment(7, 8, 5, 6, Kind::Verbatim),
                    segment(10, 16, 7, 13, Kind::Verbatim),
                ])?;
                Ok(TransformResult {
                    mapped_output: MappedOutput {
                        text: r#"add(1, 2, "oops");"#.to_string(),
                        extension: ".ts".to_string(),
                        mappings: Some(mappings),
                        ..Default::default()
                    },
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
