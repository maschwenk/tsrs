// The JSON diagnostics reply: Go's `DiagnosticResponse` (the API's wire format: UTF-16 offsets, ECMA line / UTF-16
// character positions, up to four source lines). Kept apart from tsrs_api so that the module does not link the API
// server; a test checks it against `tsrs_api::diagnostics::diagnostic_responses` byte for byte.

use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_core::collections::OrderedMap;
use tsrs_core::json::Value;
use tsrs_core::P;
use tsrs_diagnostics::Category;

fn category_number(c: Category) -> f64 {
    match c {
        Category::Warning => 0.0,
        Category::Error => 1.0,
        Category::Suggestion => 2.0,
        Category::Message => 3.0,
    }
}

struct Obj(OrderedMap<String, Value>);

impl Obj {
    fn new() -> Obj {
        Obj(OrderedMap::default())
    }

    fn set(mut self, key: &str, value: Value) -> Obj {
        self.0.insert(key.to_string(), value);
        self
    }

    fn build(self) -> Value {
        Value::Object(self.0)
    }
}

fn num(v: impl Into<f64>) -> Value {
    Value::Number(v.into())
}

fn source_lines(file: &SourceFile, first_line: i32, last_line: i32) -> Option<Value> {
    let line_map = file.ecma_line_map();
    if line_map.is_empty() {
        return None;
    }
    let lines: Vec<i32> = if last_line - first_line >= 4 {
        vec![first_line, first_line + 1, last_line - 1, last_line]
    } else {
        (first_line..=last_line).collect()
    };
    let text = file.text();
    let mut result = Vec::with_capacity(lines.len());
    for line in lines {
        let start = line_map[line as usize] as usize;
        let end = if ((line + 1) as usize) < line_map.len() { line_map[(line + 1) as usize] as usize } else { text.len() };
        result.push(Obj::new().set("line", num(line)).set("text", Value::String(text[start..end].to_string())).build());
    }
    Some(Value::Array(result))
}

fn diagnostic_response(d: &Diagnostic) -> Value {
    let file = d.file();
    let mut pos = d.pos();
    let mut end = d.end();
    if let Some(file) = file {
        let len = file.text().len() as i32;
        pos = pos.min(len).max(0);
        end = end.min(len).max(pos);
    }
    let mut o = Obj::new();
    if let Some(file) = file {
        let map = file.get_position_map();
        let (start_line, start_char) = tsrs_scanner::get_ecma_line_and_utf16_character_of_position(&*file, pos);
        let (end_line, end_char) = tsrs_scanner::get_ecma_line_and_utf16_character_of_position(&*file, end);
        o = o
            .set("fileName", Value::String(file.file_name().to_string()))
            .set("pos", num(map.utf8_to_utf16(pos)))
            .set("end", num(map.utf8_to_utf16(end)))
            .set("startPosition", Obj::new().set("line", num(start_line)).set("character", num(start_char as f64)).build())
            .set("endPosition", Obj::new().set("line", num(end_line)).set("character", num(end_char as f64)).build());
        if let Some(lines) = source_lines(&file, start_line, end_line) {
            o = o.set("sourceLines", lines);
        }
    } else {
        o = o.set("pos", num(pos)).set("end", num(end));
    }
    o = o.set("code", num(d.code())).set("category", num(category_number(d.category())));
    if !d.source().is_empty() {
        o = o.set("source", Value::String(d.source().to_string()));
    }
    o = o.set("text", Value::String(d.localize()));
    if d.reports_unnecessary() {
        o = o.set("reportsUnnecessary", Value::Bool(true));
    }
    if d.reports_deprecated() {
        o = o.set("reportsDeprecated", Value::Bool(true));
    }
    let chain = d.message_chain();
    if !chain.is_empty() {
        o = o.set("messageChain", Value::Array(chain.iter().map(|c| diagnostic_response(c)).collect()));
    }
    let related = d.related_information();
    if !related.is_empty() {
        o = o.set("relatedInformation", Value::Array(related.iter().map(|r| diagnostic_response(r)).collect()));
    }
    o.build()
}

/// The reply bytes: a JSON array of `DiagnosticResponse`s in report order.
pub fn encode(diagnostics: &[P<Diagnostic>]) -> Vec<u8> {
    let value = Value::Array(diagnostics.iter().map(|d| diagnostic_response(d)).collect());
    tsrs_core::json::marshal(&value).unwrap_or_else(|_| "[]".to_string()).into_bytes()
}
