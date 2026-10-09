use tsrs_contentmapper::{MappedOutput, ProtocolJson, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;
use tsrs_spanmap::{Feature, Kind, Segment};

use crate::protocol::{initialize_result, marshal_span_map, no_notifications, testHandler, unexpected_method, unmarshal_params};

// hoisting.go:15
pub(crate) struct hoistingHandler;

impl testHandler for hoistingHandler {}

impl ipc::Handler for hoistingHandler {
    // hoisting.go:17
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                let (text, mappings) = transform_hoisting(&p.content)?;
                Ok(TransformResult {
                    mapped_output: MappedOutput { text, extension: ".ts".to_string(), mappings: Some(mappings), ..Default::default() },
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

// transformHoisting emits the script body inside a render function, but lifts the leading run of import
// declarations above it, so the virtual text is laid out as:
//
//	///<reference types="svelte" />
//	;
//	import { existing } from "./dep";      <- original [importsStart, importsEnd)
//	function $$render() {
//	<whitespace before the first import>   <- original [scriptStart, importsStart)
//	<script body after the last import>    <- original [importsEnd, scriptEnd)
//	}
//
// This mirrors the layout svelte2tsx produces for a component. Hoisting splits the script into segments
// that are contiguous in the original but disjoint in the virtual text, so the original position at a
// splice point (importsStart) lies on the boundary of two verbatim segments and projects to two distinct
// exact virtual positions.
// hoisting.go:51
fn transform_hoisting(content: &str) -> Result<(String, Value), ipc::Error> {
    let mut virtual_ = String::new();
    let mut segments = Vec::new();

    let mut write_mapped = |virtual_: &mut String, original_start: usize, original_end: usize| {
        let virtual_start = virtual_.len() as i32;
        virtual_.push_str(&content[original_start..original_end]);
        segments.push(Segment {
            virtual_start,
            virtual_end: virtual_.len() as i32,
            original_start: original_start as i32,
            original_end: original_end as i32,
            kind: Kind::Verbatim,
            features: Feature::All,
        });
    };

    let (script_start, script_end) = script_range(content)?;
    let (imports_start, imports_end) = leading_import_range(content, script_start, script_end);

    virtual_.push_str("///<reference types=\"svelte\" />\n;\n");
    write_mapped(&mut virtual_, imports_start, imports_end);
    virtual_.push_str("\nfunction $$render() {");
    write_mapped(&mut virtual_, script_start, imports_start);
    write_mapped(&mut virtual_, imports_end, script_end);
    virtual_.push_str("\n;\nreturn { props: {} as Record<string, never> }}\n");

    let mappings = marshal_span_map(&segments)?;
    Ok((virtual_, mappings))
}

// hoisting.go:88
fn script_range(content: &str) -> Result<(usize, usize), ipc::Error> {
    let Some(script_open) = content.find("<script") else {
        return Err(ipc::Error::new("contentmappertest: missing <script> tag"));
    };
    let Some(open_end_rel) = content[script_open..].find('>') else {
        return Err(ipc::Error::new("contentmappertest: unclosed <script> tag"));
    };
    let start = script_open + open_end_rel + 1;
    let Some(close_rel) = content[start..].find("</script>") else {
        return Err(ipc::Error::new("contentmappertest: missing </script> tag"));
    };
    Ok((start, start + close_rel))
}

// leadingImportRange returns the range covering the run of import declarations at the top of the script
// body, skipping the whitespace that precedes them.
// hoisting.go:107
fn leading_import_range(content: &str, script_start: usize, script_end: usize) -> (usize, usize) {
    let bytes = content.as_bytes();
    let mut start = script_start;
    while start < script_end && is_space_byte(bytes[start]) {
        start += 1;
    }
    let mut end = start;
    while end < script_end && content[end..script_end].starts_with("import ") {
        let Some(line_end) = content[end..script_end].find('\n') else {
            end = script_end;
            break;
        };
        end += line_end;
        while end < script_end && is_space_byte(bytes[end]) {
            end += 1;
        }
    }
    while end > start && is_space_byte(bytes[end - 1]) {
        end -= 1;
    }
    (start, end)
}

// hoisting.go:130
fn is_space_byte(ch: u8) -> bool {
    ch == b' ' || ch == b'\t' || ch == b'\r' || ch == b'\n'
}
