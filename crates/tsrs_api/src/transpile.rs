// Port of tsc/internal/transpile (transpile.go, fs.go) plus session.go handleTranspile /
// handleTranspileFromFile / transpileOutput.
//
// Memory: each call builds a one-file program that is not freed (same lifetime model as the CLI; see the
// "Known gaps" section of docs/NODE_API.md).

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rustc_hash::FxHashMap;
use tsrs_compiler::{EmitOnly, EmitOptions, ProgramOptions, WriteFileData};
use tsrs_core::context::Context;
use tsrs_core::json::Value;
use tsrs_core::{tspath, CompilerOptions, JsxEmit, Tristate, P};
use tsrs_tsoptions::gojson;
use tsrs_vfs::{Entries, FileInfo, FS};

use crate::diagnostics::diagnostic_responses;
use crate::handler::{ApiError, ApiResult};
use crate::session::Session;
use crate::wire::{s, Obj, Params};

const INPUT_DIRECTORY: &str = "/";
const LIB_DIRECTORY: &str = "/lib";

const BAREBONES_LIB_CONTENT: &str = "interface Boolean {}
interface Function {}
interface CallableFunction {}
interface NewableFunction {}
interface IArguments {}
interface Number {}
interface Object {}
interface RegExp {}
interface String {}
interface Array<T> { length: number; [n: number]: T; }
interface SymbolConstructor {
    (desc?: string | number): symbol;
    for(name: string): symbol;
    readonly toStringTag: symbol;
}
declare var Symbol: SymbolConstructor;
interface Symbol {
    readonly [Symbol.toStringTag]: string;
}";

/// Go `transpileFS`: only the synthesized files exist. Go panics on any other access; here unexpected
/// access is recorded and turned into an error after the program is built.
struct TranspileFS {
    files: FxHashMap<String, String>,
    unexpected: Mutex<Option<String>>,
}

impl TranspileFS {
    fn note(&self, what: &str, path: &str) {
        let mut u = self.unexpected.lock().unwrap();
        if u.is_none() {
            *u = Some(format!("unexpected {what} for {path:?}"));
        }
    }
}

impl FS for TranspileFS {
    fn use_case_sensitive_file_names(&self) -> bool {
        true
    }
    fn file_exists(&self, path: &str) -> bool {
        let ok = self.files.contains_key(path);
        if !ok {
            self.note("file existence check", path);
        }
        ok
    }
    fn read_file(&self, path: &str) -> Option<String> {
        let r = self.files.get(path).cloned();
        if r.is_none() {
            self.note("file read", path);
        }
        r
    }
    fn write_file(&self, path: &str, _data: &str) -> Result<(), String> {
        Err(format!("unexpected write for {path:?}"))
    }
    fn append_file(&self, path: &str, _data: &str) -> Result<(), String> {
        Err(format!("unexpected append for {path:?}"))
    }
    fn remove(&self, path: &str) -> Result<(), String> {
        Err(format!("unexpected remove for {path:?}"))
    }
    fn chtimes(&self, path: &str, _a: SystemTime, _m: SystemTime) -> Result<(), String> {
        Err(format!("unexpected chtimes for {path:?}"))
    }
    fn directory_exists(&self, path: &str) -> bool {
        self.note("directory existence check", path);
        false
    }
    fn get_accessible_entries(&self, path: &str) -> Entries {
        self.note("directory listing", path);
        Entries::default()
    }
    fn stat(&self, path: &str) -> Option<FileInfo> {
        self.note("stat", path);
        None
    }
    fn realpath(&self, path: &str) -> String {
        self.note("realpath request", path);
        path.to_string()
    }
}

pub struct TranspileOutput {
    pub output_text: String,
    pub diagnostics: Vec<P<tsrs_ast::Diagnostic>>,
    pub source_map_text: String,
}

/// Go `transpileWorker`.
pub fn transpile(input: &str, base: Option<&CompilerOptions>, file_name: &str, report_diagnostics: bool, declaration: bool) -> ApiResult<TranspileOutput> {
    let mut opts = base.cloned().unwrap_or_default();
    opts.incremental = Tristate::Unknown;
    opts.declaration = Tristate::Unknown;
    opts.emit_declaration_only = Tristate::Unknown;
    opts.no_emit = Tristate::Unknown;
    opts.lib = None;
    opts.out_file = String::new();
    opts.composite = Tristate::Unknown;
    opts.ts_build_info_file = String::new();
    opts.paths = None;
    opts.root_dirs = None;
    opts.types = None;
    opts.allow_importing_ts_extensions = Tristate::Unknown;
    opts.no_emit_on_error = Tristate::Unknown;
    opts.declaration_dir = String::new();
    if !opts.verbatim_module_syntax.is_true() {
        opts.isolated_modules = Tristate::True;
    }
    opts.no_check = Tristate::True;
    opts.no_resolve = Tristate::True;
    opts.suppress_output_path_check = Tristate::True;
    opts.allow_non_ts_extensions = Tristate::True;
    if declaration {
        opts.declaration = Tristate::True;
        opts.emit_declaration_only = Tristate::True;
        opts.isolated_declarations = Tristate::True;
        opts.no_lib = Tristate::False;
    } else {
        opts.declaration = Tristate::False;
        opts.declaration_map = Tristate::False;
        opts.isolated_declarations = Tristate::False;
        opts.no_lib = Tristate::True;
    }

    let file_name = if file_name.is_empty() {
        if opts.jsx != JsxEmit::None {
            "module.tsx"
        } else {
            "module.ts"
        }
    } else {
        file_name
    };
    let input_file_name = tspath::get_normalized_absolute_path(file_name, INPUT_DIRECTORY);
    let mut files = FxHashMap::default();
    files.insert(input_file_name.clone(), input.to_string());
    if declaration {
        let lib = tsrs_tsoptions::get_default_lib_file_name(&opts);
        files.insert(tspath::combine_paths(LIB_DIRECTORY, &[lib]), BAREBONES_LIB_CONTENT.to_string());
    }
    let fs = Arc::new(TranspileFS { files, unexpected: Mutex::new(None) });
    let host = tsrs_compiler::new_compiler_host(INPUT_DIRECTORY, fs.clone(), LIB_DIRECTORY, None, None);
    let config = tsrs_tsoptions::new_parsed_command_line(P::new(opts), vec![input_file_name.clone()], Vec::new(), tspath::ComparePathsOptions::default());
    let mut program_options = ProgramOptions::new(P::new(config), host);
    program_options.skip_module_resolution = true;
    let program = tsrs_compiler::new_program(program_options);

    let ctx = Context::background();
    let mut all = Vec::new();
    if report_diagnostics {
        let source_file = program.get_source_file(&input_file_name);
        all.extend(program.get_syntactic_diagnostics(&ctx, source_file));
        all.extend(program.get_config_file_parsing_diagnostics());
        all.extend(program.get_program_diagnostics());
    }
    let outputs: Mutex<(Option<String>, Option<String>)> = Mutex::new((None, None));
    let write = |name: &str, text: &str, _data: &mut WriteFileData| -> Result<(), String> {
        let mut o = outputs.lock().unwrap();
        let slot = if name.ends_with(".map") { &mut o.1 } else { &mut o.0 };
        if slot.is_some() {
            return Err(format!("Unexpected multiple outputs, file: {name}"));
        }
        *slot = Some(text.to_string());
        Ok(())
    };
    let emit_only = if declaration { EmitOnly::EmitOnlyDts } else { EmitOnly::EmitAll };
    let result = program.emit(&ctx, EmitOptions { target_source_files: None, emit_only, force_emit: declaration, write_file: Some(&write) });
    if let Some(err) = fs.unexpected.lock().unwrap().take() {
        return Err(ApiError::internal(format!("transpile: {err}")));
    }
    all.extend(result.diagnostics);
    let (output_text, source_map_text) = outputs.into_inner().unwrap();
    let output_text = output_text.ok_or_else(|| ApiError::internal("transpile: Output generation failed"))?;
    Ok(TranspileOutput { output_text, diagnostics: all, source_map_text: source_map_text.unwrap_or_default() })
}

fn transpile_response(out: TranspileOutput) -> Value {
    Obj::new()
        .set("outputText", s(out.output_text))
        .set_opt("diagnostics", (!out.diagnostics.is_empty()).then(|| diagnostic_responses(&out.diagnostics)))
        .set_opt("sourceMapText", (!out.source_map_text.is_empty()).then(|| s(out.source_map_text)))
        .build()
}

struct ParsedTranspileOptions {
    compiler_options: Option<CompilerOptions>,
    file_name: String,
    report_diagnostics: bool,
}

fn parse_options(v: &Value) -> ApiResult<ParsedTranspileOptions> {
    let p = Params(v);
    let compiler_options = if p.has("compilerOptions") {
        Some(gojson::compiler_options_from_go_json(p.get("compilerOptions")).map_err(ApiError::invalid_request)?)
    } else {
        None
    };
    Ok(ParsedTranspileOptions { compiler_options, file_name: p.str("fileName")?.to_string(), report_diagnostics: p.bool("reportDiagnostics")? })
}

impl Session {
    pub(crate) fn handle_transpile(&self, p: Params, declaration: bool) -> ApiResult<Value> {
        let o = parse_options(p.get("options"))?;
        let out = transpile(p.str("input")?, o.compiler_options.as_ref(), &o.file_name, o.report_diagnostics, declaration)?;
        Ok(transpile_response(out))
    }

    pub(crate) fn handle_transpile_from_file(&self, p: Params, declaration: bool) -> ApiResult<Value> {
        let file_name = tspath::get_normalized_absolute_path(p.str("fileName")?, self.current_directory());
        let input = self.base_fs().read_file(&file_name).ok_or_else(|| ApiError::client(format!("could not read file {file_name:?}")))?;
        let o = parse_options(p.get("options"))?;
        let out = transpile(&input, o.compiler_options.as_ref(), &file_name, o.report_diagnostics, declaration)?;
        Ok(transpile_response(out))
    }
}
