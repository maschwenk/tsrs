use tsrs_contentmapper::{MappedOutput, ProtocolJson, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;
use tsrs_spanmap::{Feature, Kind, Segment};

use crate::protocol::{initialize_result, marshal_span_map, no_notifications, testHandler, unexpected_method, unmarshal_params};

// duplicate.go:14
pub(crate) struct duplicateHandler;

impl testHandler for duplicateHandler {}

impl ipc::Handler for duplicateHandler {
    // duplicate.go:16
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                let content = p.content.as_str();
                let length = content.len();
                // A verbatim segment of the whole original content at virtual position `virtual_start`.
                let segment = |virtual_start: usize, features: Feature| Segment {
                    virtual_start: virtual_start as u32,
                    virtual_end: (virtual_start + length) as u32,
                    original_start: 0,
                    original_end: length as u32,
                    kind: Kind::Verbatim,
                    features,
                };
                let result = |virtual_: String, mappings: Value| {
                    TransformResult {
                        mapped_output: MappedOutput { text: virtual_, extension: ".ts".to_string(), mappings: Some(mappings), ..Default::default() },
                        ..Default::default()
                    }
                    .marshal_json()
                };
                if p.file_name.contains("hover-fallback") {
                    let virtual_ = format!("// {content}\nconst {content} = 1;\n");
                    let first = "// ".len();
                    let second = first + length + "\nconst ".len();
                    let mappings = marshal_span_map(&[
                        segment(first, Feature::Hover),
                        segment(second, Feature::Hover),
                    ])?;
                    return Ok(result(virtual_, mappings));
                }
                if p.file_name.contains("hover-concat") {
                    let virtual_ = format!("namespace A {{ export const {content} = 1; }}\nnamespace B {{ export const {content} = \"text\"; }}\n");
                    let first = index(&virtual_, content);
                    let second = last_index(&virtual_, content);
                    let mappings = marshal_span_map(&[segment(first, Feature::Hover), segment(second, Feature::Hover)])?;
                    return Ok(result(virtual_, mappings));
                }
                if p.file_name.contains("signature-fallback") {
                    let virtual_ = format!(
                        "// {content}\nfunction use(value: number): void {{}}\n{content};\n"
                    );
                    let first = "// ".len();
                    let second = last_index(&virtual_, content);
                    let mappings =
                        marshal_span_map(&[segment(first, Feature::SignatureHelp), segment(second, Feature::SignatureHelp)])?;
                    return Ok(result(virtual_, mappings));
                }
                if p.file_name.contains("rename-conflict") {
                    let virtual_ = format!("export const {content} = 1;\nconst object = {{ {content} }};\n{content};\n");
                    let first = index(&virtual_, content);
                    let second = index(&virtual_[(first + length) as usize..], content) + first + length;
                    let third = index(&virtual_[(second + length) as usize..], content) + second + length;
                    let mappings = marshal_span_map(&[
                        segment(first, Feature::Rename),
                        segment(second, Feature::Rename),
                        segment(third, Feature::Rename),
                    ])?;
                    return Ok(result(virtual_, mappings));
                }
                let virtual_ = format!("export const {content} = 1;\n{content};\n");
                let first = "export const ".len();
                let second = first + length + " = 1;\n".len();
                let disabled = p.file_name.contains("disabled");
                let mut semantic_features = Feature::Hover | Feature::Definition | Feature::References | Feature::Rename;
                let mut navigation_features = Feature::Definition | Feature::References | Feature::Rename;
                if disabled {
                    semantic_features = Feature::None;
                    navigation_features = Feature::None;
                }
                let mappings = marshal_span_map(&[segment(first, semantic_features), segment(second, navigation_features)])?;
                Ok(result(virtual_, mappings))
            }
            _ => Err(unexpected_method(method)),
        }
    }

    fn handle_notification(&self, _method: &str, _params: Option<&Value>) -> Result<(), ipc::Error> {
        no_notifications()
    }
}

fn index(s: &str, substr: &str) -> usize {
    s.find(substr)
        .expect("constructed virtual text contains the requested substring")
}

fn last_index(s: &str, substr: &str) -> usize {
    s.rfind(substr)
        .expect("constructed virtual text contains the requested substring")
}
