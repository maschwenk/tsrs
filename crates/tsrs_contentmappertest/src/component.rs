use tsrs_contentmapper::{MappedOutput, ProtocolJson, TransformParams, TransformResult, METHOD_INITIALIZE, METHOD_TRANSFORM};
use tsrs_core::json::Value;
use tsrs_ipc as ipc;
use tsrs_spanmap::{Feature, Kind, Segment};

use crate::protocol::{initialize_result, marshal_span_map, no_notifications, testHandler, unexpected_method, unmarshal_params};

// component.go:15
pub(crate) struct componentHandler;

impl testHandler for componentHandler {}

impl ipc::Handler for componentHandler {
    // component.go:17
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, ipc::Error> {
        match method {
            METHOD_INITIALIZE => Ok(initialize_result("mapper").marshal_json()),
            METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(params)?;
                let (text, mappings) = transform_component(&p.content)?;
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

// Go's writeSynthesized, writeMapped and writeAnchored closures share the builder and the segments, which
// `componentWriter` holds.
struct componentWriter {
    virtual_: String,
    segments: Vec<Segment>,
}

impl componentWriter {
    // component.go:40
    fn write_synthesized(&mut self, text: &str) {
        self.virtual_.push_str(text);
    }

    // component.go:43
    fn write_mapped(&mut self, text: &str, original_start: usize, original_end: usize, kind: Kind) {
        let virtual_start = self.virtual_.len() as i32;
        self.virtual_.push_str(text);
        self.segments.push(Segment {
            virtual_start,
            virtual_end: self.virtual_.len() as i32,
            original_start: original_start as i32,
            original_end: original_end as i32,
            kind,
            features: Feature::All,
        });
    }

    // component.go:55
    fn write_anchored(&mut self, text: &str, original_position: usize, features: Feature) {
        let virtual_start = self.virtual_.len() as i32;
        self.virtual_.push_str(text);
        self.segments.push(Segment {
            virtual_start,
            virtual_end: self.virtual_.len() as i32,
            original_start: original_position as i32,
            original_end: original_position as i32,
            kind: Kind::Atom,
            features,
        });
    }
}

// component.go:36
fn transform_component(content: &str) -> Result<(String, Value), ipc::Error> {
    let mut w = componentWriter { virtual_: String::new(), segments: Vec::new() };

    if let Some(script_open) = content.find("<script") {
        let Some(open_end_rel) = content[script_open..].find('>') else {
            return Err(ipc::Error::new("contentmappertest: unclosed <script> tag"));
        };
        let script_start = script_open + open_end_rel + 1;
        let Some(close_rel) = content[script_start..].find("</script>") else {
            return Err(ipc::Error::new("contentmappertest: missing </script> tag"));
        };
        let script_end = script_start + close_rel;
        w.write_mapped(&content[script_start..script_end], script_start, script_end, Kind::Verbatim);
    }

    w.write_synthesized("\nfunction __render() {\n");
    let bytes = content.as_bytes();
    let mut search_start = 0;
    while search_start < content.len() {
        let Some(open_rel) = content[search_start..].find("{{") else {
            break;
        };
        let expr_start = search_start + open_rel + "{{".len();
        let Some(close_rel) = content[expr_start..].find("}}") else {
            return Err(ipc::Error::new("contentmappertest: unclosed template expression"));
        };
        let expr_end = expr_start + close_rel;
        w.write_synthesized("  void (");
        let mut pos = expr_start;
        while pos < expr_end {
            if !is_identifier_start(bytes[pos]) {
                w.write_synthesized(&content[pos..pos + 1]);
                pos += 1;
                continue;
            }
            let mut end = pos + 1;
            while end < expr_end && is_identifier_part(bytes[end]) {
                end += 1;
            }
            w.write_mapped(&content[pos..end], pos, end, Kind::Atom);
            pos = end;
        }
        w.write_synthesized(");\n");
        search_start = expr_end + "}}".len();
    }
    w.write_synthesized("}\n");
    if let Some((name_start, name_end)) = component_name_range(content) {
        w.write_synthesized("export class ");
        w.write_mapped(&content[name_start..name_end], name_start, name_end, Kind::Atom);
        w.write_synthesized(" {}\n");
    }
    w.write_anchored("export default {};\n", 0, Feature::Definition | Feature::References);

    let mappings = marshal_span_map(&w.segments)?;
    Ok((w.virtual_, mappings))
}

// component.go:127
fn component_name_range(content: &str) -> Option<(usize, usize)> {
    let component_start = content.find("<component")?;
    let tag_end_rel = content[component_start..].find('>')?;
    let tag = &content[component_start..component_start + tag_end_rel];
    let name_rel = tag.find(r#"name=""#)?;
    let start = component_start + name_rel + r#"name=""#.len();
    let end_rel = content[start..].find('"')?;
    Some((start, start + end_rel))
}

// component.go:149
fn is_identifier_start(ch: u8) -> bool {
    ch == b'_' || ch == b'$' || ch.is_ascii_uppercase() || ch.is_ascii_lowercase()
}

// component.go:153
fn is_identifier_part(ch: u8) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}
