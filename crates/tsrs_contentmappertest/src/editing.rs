use tsrs_contentmapper::{MappedOutput, ProtocolJson, SupplementalOutput, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;
use tsrs_spanmap::{Feature, Kind, Segment};

use crate::protocol::{initialize_result, marshal_span_map, no_notifications, testHandler, unexpected_method, unmarshal_params};

// editing.go:15
pub(crate) struct prefixedSupplementalHandler;

impl testHandler for prefixedSupplementalHandler {}

// A verbatim segment with every feature, the shape most of editing.go's segments have.
fn verbatim_segment(virtual_start: usize, virtual_end: usize, original_start: usize, original_end: usize) -> Segment {
    Segment {
        virtual_start: virtual_start as i32,
        virtual_end: virtual_end as i32,
        original_start: original_start as i32,
        original_end: original_end as i32,
        kind: Kind::Verbatim,
        features: Feature::All,
    }
}

impl ipc::Handler for prefixedSupplementalHandler {
    // editing.go:17
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                let content = p.content.as_str();
                const PREFIX: &str = "/* generated */\n";
                let mut features = Feature::All;
                if p.file_name.contains("folding-disabled")
                    || p.file_name.contains("codelens-disabled")
                    || p.file_name.contains("formatting-disabled")
                {
                    features = Feature::None;
                }
                let mut supplemental_text = format!("{PREFIX}{content}");
                let mut segments = vec![Segment {
                    features,
                    ..verbatim_segment(PREFIX.len(), PREFIX.len() + content.len(), 0, content.len())
                }];
                if p.file_name.contains("formatting-split") {
                    let Some(second_start) = content.find("function second") else {
                        return Err(ipc::Error::new("contentmappertest: formatting-split input is missing function second"));
                    };
                    const GENERATED: &str = "const generated={x:1};\n";
                    supplemental_text = format!("{PREFIX}{}{GENERATED}{}", &content[..second_start], &content[second_start..]);
                    segments = vec![
                        verbatim_segment(PREFIX.len(), PREFIX.len() + second_start, 0, second_start),
                        verbatim_segment(
                            PREFIX.len() + second_start + GENERATED.len(),
                            supplemental_text.len(),
                            second_start,
                            content.len(),
                        ),
                    ];
                }
                if p.file_name.contains("formatting-overlap") {
                    let (Some(second_start), Some(third_start)) = (content.find("function second"), content.find("function third")) else {
                        return Err(ipc::Error::new(
                            "contentmappertest: formatting-overlap input is missing function second or third",
                        ));
                    };
                    const WRAPPER_START: &str = "if (true) {\n";
                    const WRAPPER_END: &str = "}\n";
                    supplemental_text = format!("{WRAPPER_START}{}{WRAPPER_END}", &content[second_start..]);
                    segments = vec![verbatim_segment(
                        WRAPPER_START.len(),
                        WRAPPER_START.len() + content.len() - second_start,
                        second_start,
                        content.len(),
                    )];
                    let canonical_mappings = marshal_span_map(&[verbatim_segment(0, third_start, 0, third_start)])?;
                    let mappings = marshal_span_map(&segments)?;
                    return Ok(TransformResult {
                        mapped_output: MappedOutput {
                            text: content[..third_start].to_string(),
                            extension: ".ts".to_string(),
                            mappings: Some(canonical_mappings),
                            ..Default::default()
                        },
                        supplemental: vec![SupplementalOutput {
                            mapped_output: MappedOutput {
                                text: supplemental_text,
                                extension: ".ts".to_string(),
                                mappings: Some(mappings),
                                ..Default::default()
                            },
                        }],
                        ..Default::default()
                    }
                    .marshal_json());
                }
                let mappings = marshal_span_map(&segments)?;
                let mut canonical = MappedOutput { text: "export {};".to_string(), extension: ".ts".to_string(), ..Default::default() };
                if p.file_name.contains("folding-duplicate")
                    || p.file_name.contains("codelens-disabled")
                    || p.file_name.contains("codelens-duplicate")
                {
                    let canonical_mappings = marshal_span_map(&[verbatim_segment(0, content.len(), 0, content.len())])?;
                    canonical = MappedOutput {
                        text: content.to_string(),
                        extension: ".ts".to_string(),
                        mappings: Some(canonical_mappings),
                        ..Default::default()
                    };
                }
                Ok(TransformResult {
                    mapped_output: canonical,
                    supplemental: vec![SupplementalOutput {
                        mapped_output: MappedOutput {
                            text: supplemental_text,
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

// editing.go:139
pub(crate) struct unmappedFoldingHandler;

impl testHandler for unmappedFoldingHandler {}

impl ipc::Handler for unmappedFoldingHandler {
    // editing.go:141
    fn handle_request(&self, method: &str, _params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let mappings = marshal_span_map(&[])?;
                Ok(TransformResult {
                    mapped_output: MappedOutput {
                        text: "import \"a\";\nimport \"b\";\n/*\n * generated\n */\nexport {};".to_string(),
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
