// session.go handleParseCommandLine / handleReadConfigFile / handleParseJsonConfigFileContent /
// handleParseConfigFile and proto.go NewConfigFileResponse / toProtocolJSONValue / jsonValueToAny.

use std::sync::Arc;

use tsrs_core::collections::OrderedMap;
use tsrs_core::json::Value;
use tsrs_core::tspath;
use tsrs_tsoptions::gojson;
use tsrs_tsoptions::{CompilerOptionsValue, ParseConfigHost, ParsedCommandLine};
use tsrs_vfs::FS;

use crate::diagnostics::{diagnostic_response, diagnostic_responses};
use crate::handler::{ApiError, ApiResult};
use crate::session::Session;
use crate::wire::{strings, DocumentIdentifier, Obj, Params};

/// `tsoptions.ParseConfigHost` over the session's base filesystem (Go passes the SnapshotHost).
pub(crate) struct ApiParseConfigHost {
    pub fs: Arc<dyn FS>,
    pub cwd: String,
}

impl ParseConfigHost for ApiParseConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
}

/// Go `toProtocolJSONValue` composed with `json.Marshal` of the resulting `any`.
pub fn options_value_to_json(v: &CompilerOptionsValue) -> Value {
    match v {
        CompilerOptionsValue::Null | CompilerOptionsValue::NilArray => Value::Null,
        CompilerOptionsValue::Bool(b) => Value::Bool(*b),
        CompilerOptionsValue::Float(f) => Value::Number(*f),
        CompilerOptionsValue::Int(i) => Value::Number(*i as f64),
        CompilerOptionsValue::String(s) => Value::String(s.clone()),
        CompilerOptionsValue::Array(a) => Value::Array(a.iter().map(options_value_to_json).collect()),
        CompilerOptionsValue::StringArray(a) => strings(a.iter().cloned()),
        CompilerOptionsValue::Object(m) => {
            let mut o = OrderedMap::default();
            for (k, v) in m.iter() {
                o.insert(k.clone(), options_value_to_json(v));
            }
            Value::Object(o)
        }
        CompilerOptionsValue::EmptyStruct => Value::Object(OrderedMap::default()),
        CompilerOptionsValue::Tristate(t) => match t {
            tsrs_core::Tristate::Unknown => Value::Null,
            tsrs_core::Tristate::False => Value::Bool(false),
            tsrs_core::Tristate::True => Value::Bool(true),
        },
        CompilerOptionsValue::ModuleKind(x) => Value::Number(x.value() as f64),
        CompilerOptionsValue::ModuleResolutionKind(x) => Value::Number(*x as i32 as f64),
        CompilerOptionsValue::ModuleDetectionKind(x) => Value::Number(*x as i32 as f64),
        CompilerOptionsValue::ScriptTarget(x) => Value::Number(x.value() as f64),
        CompilerOptionsValue::JsxEmit(x) => Value::Number(x.value() as f64),
        CompilerOptionsValue::NewLineKind(x) => Value::Number(*x as i32 as f64),
        // toProtocolJSONValue: watch enums are shifted to the public (TS5) numbering.
        CompilerOptionsValue::WatchFileKind(x) => Value::Number((*x as i32 - 1) as f64),
        CompilerOptionsValue::WatchDirectoryKind(x) => Value::Number((*x as i32 - 1) as f64),
        CompilerOptionsValue::PollingKind(x) => Value::Number((*x as i32 - 1) as f64),
    }
}

/// Go `jsonValueToAny`: numbers are float64, arrays `[]any`, objects ordered maps.
pub fn json_to_options_value(v: &Value) -> CompilerOptionsValue {
    match v {
        Value::Null => CompilerOptionsValue::Null,
        Value::Bool(b) => CompilerOptionsValue::Bool(*b),
        Value::Integer(n) => CompilerOptionsValue::Float(*n as f64),
        Value::Number(n) => CompilerOptionsValue::Float(*n),
        Value::String(s) => CompilerOptionsValue::String(s.clone()),
        Value::Array(a) => CompilerOptionsValue::Array(a.iter().map(json_to_options_value).collect()),
        Value::Object(o) => {
            let mut m = OrderedMap::default();
            for (k, v) in o.iter() {
                m.insert(k.clone(), json_to_options_value(v));
            }
            CompilerOptionsValue::Object(m)
        }
    }
}

/// Go `NewConfigFileResponse`.
pub fn config_file_response(parsed: &ParsedCommandLine) -> Value {
    let mut compile_on_save = parsed.compile_on_save;
    if compile_on_save.is_none() {
        if let CompilerOptionsValue::Object(raw) = &parsed.raw {
            if let Some(CompilerOptionsValue::Bool(b)) = raw.get("compileOnSave") {
                compile_on_save = Some(*b);
            }
        }
    }
    let options = match parsed.compiler_options() {
        Some(o) => gojson::compiler_options_to_go_json(&o),
        None => Value::Null,
    };
    let refs = parsed.project_references();
    let raw = options_value_to_json(&parsed.raw);
    Obj::new()
        .set("fileNames", strings(parsed.file_names().iter().cloned()))
        .set("options", options)
        .set_omitempty("projectReferences", Value::Array(refs.iter().map(gojson::project_reference_to_go_json).collect()))
        .set_omitempty("typeAcquisition", parsed.type_acquisition().map(gojson::type_acquisition_to_go_json).unwrap_or(Value::Null))
        .set_omitempty("compileOnSave", compile_on_save.map(Value::Bool).unwrap_or(Value::Null))
        .set_omitempty("raw", raw)
        .set("errors", diagnostic_responses(&parsed.errors))
        .build()
}

impl Session {
    pub(crate) fn handle_parse_command_line(&self, p: Params) -> ApiResult<Value> {
        let command_line = p.strings("commandLine")?;
        Ok(config_file_response(&tsrs_tsoptions::parse_command_line(&command_line, self.parse_config_host)))
    }

    pub(crate) fn handle_read_config_file(&self, p: Params) -> ApiResult<Value> {
        let file = p.document("file")?;
        let config_file_name = file.to_absolute_file_name(self.current_directory());
        let Some(content) = self.base_fs().read_file(&config_file_name) else {
            let diag = tsrs_ast::new_compiler_diagnostic(&tsrs_diagnostics::Cannot_read_file_0, &[&config_file_name]);
            return Ok(Obj::new().set("config", Value::Object(OrderedMap::default())).set("error", diagnostic_response(&diag)).build());
        };
        let (config, errors) = tsrs_tsoptions::parse_config_file_text_to_json(&config_file_name, self.to_path(&config_file_name), &content);
        Ok(Obj::new()
            .set("config", options_value_to_json(&config))
            .set_opt("error", errors.first().map(|d| diagnostic_response(d)))
            .build())
    }

    pub(crate) fn handle_parse_json_config_file_content(&self, p: Params) -> ApiResult<Value> {
        let has_dir = p.has("configDirectory");
        let has_name = p.has("configFileName");
        if has_dir == has_name {
            return Err(ApiError::client("exactly one of configDirectory or configFileName is required"));
        }
        let cwd = self.current_directory();
        let (base_path, config_file_name) = if has_dir {
            (tspath::get_normalized_absolute_path(p.str("configDirectory")?, cwd), String::new())
        } else {
            let name = p.document("configFileName")?.to_absolute_file_name(cwd);
            (tspath::get_directory_path(&name).to_string(), name)
        };
        let parsed = tsrs_tsoptions::parse_json_config_file_content(
            json_to_options_value(p.get("json")),
            self.parse_config_host,
            &base_path,
            None,
            &config_file_name,
            &[],
            None,
        );
        Ok(config_file_response(&parsed))
    }

    pub(crate) fn handle_parse_config_file(&self, p: Params) -> ApiResult<Value> {
        let file = p.document("file")?;
        let config_file_name = file.to_absolute_file_name(self.current_directory());
        let Some(content) = self.base_fs().read_file(&config_file_name) else {
            return Err(ApiError::client(format!("could not read file {config_file_name:?}")));
        };
        let config_dir = tspath::get_directory_path(&config_file_name).to_string();
        let source = tsrs_tsoptions::new_tsconfig_source_file_from_file_path(&config_file_name, self.to_path(&config_file_name), &content);
        let parsed = tsrs_tsoptions::parse_json_source_file_config_file_content(
            source,
            self.parse_config_host,
            &config_dir,
            None,
            None,
            &config_file_name,
            &[],
            None,
        );
        Ok(config_file_response(&parsed))
    }
}
