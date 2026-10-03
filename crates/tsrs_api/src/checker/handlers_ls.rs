// Language-service backed handlers (Go tsc/internal/api/session.go: handleGetSignatureUsages,
// handleGetCompletionsAtPosition, handleGetReferencedSymbolsForNode, handleGetImportAdderEdits).
//
// Like Go these do not take the persistent API checker for the whole request: the language service
// acquires its own checkers (find-all-references acquires several, which would deadlock on the
// single-slot API checker). Only completions with `includeSymbol` use the API checker lifetime, so the
// returned symbols stay resolvable by later checker requests (Go does the same).

use std::sync::Arc;

use tsrs_ast::Symbol;
use tsrs_compiler::Program;
use tsrs_core::context::{with_checker_lifetime, CheckerLifetime, Context};
use tsrs_core::json::Value;
use tsrs_core::P;
use tsrs_ls::autoimport::ProjectID as LsProjectID;
use tsrs_project::Snapshot;

use super::host::{CheckerError, CheckerHost, CheckerResult};
use super::json::obj;
use super::params::{Params, SymbolReference};
use super::setup::{resolve_symbol_for_program, SnapshotCtx};

fn snapshot_ctx<'h>(host: &'h dyn CheckerHost, p: &Params) -> CheckerResult<SnapshotCtx<'h>> {
    SnapshotCtx::new(host, p.u64("snapshot")?, p.project()?)
}

fn program_of(snapshot: &Snapshot, project: &str) -> CheckerResult<&'static Program> {
    let proj = snapshot
        .project_collection
        .get_project(&tsrs_project::ID(project.to_string()))
        .ok_or_else(|| CheckerError::client(format!("project {project} not found")))?;
    proj.get_program().ok_or_else(|| CheckerError::client("project has no program"))
}

/// Go `setupLanguageService`.
fn language_service(snapshot: &Arc<Snapshot>, program: &'static Program, project: &str, active_file: &str) -> CheckerResult<tsrs_ls::LanguageService> {
    if snapshot.project_collection.get_project(&tsrs_project::ID(project.to_string())).is_none() {
        return Err(CheckerError::client(format!("project {project} not found")));
    }
    let host: Arc<dyn tsrs_ls::Host> = snapshot.clone();
    Ok(tsrs_ls::new_language_service(LsProjectID(project.to_string()), program, host, active_file))
}

pub(crate) fn get_signature_usages(host: &dyn CheckerHost, p: &Params) -> CheckerResult<Value> {
    let sd = snapshot_ctx(host, p)?;
    let program = sd.program()?;
    let decl = host.resolve_node_handle(program, p.string("signatureDecl")?)?;
    let ls = language_service(&sd.scope.snapshot, program, &sd.project, "")?;
    let usages = ls.get_signature_usages(&host.context(), decl);
    if usages.is_empty() {
        return Ok(Value::Null);
    }
    let mut out = Vec::with_capacity(usages.len());
    for usage in usages {
        let mut o = obj();
        o.set("name", Value::String(host.node_handle(usage.name)?));
        if let Some(call) = usage.call {
            o.set("call", Value::String(host.node_handle(call)?));
        }
        out.push(o.build());
    }
    Ok(Value::Array(out))
}

pub(crate) fn get_referenced_symbols_for_node(host: &dyn CheckerHost, p: &Params) -> CheckerResult<Value> {
    let sd = snapshot_ctx(host, p)?;
    let program = sd.program()?;
    let node = host.resolve_node_handle(program, p.string("node")?)?;
    let position = p.i32("position")?;
    let ls = language_service(&sd.scope.snapshot, program, &sd.project, "")?;
    let entries = ls.get_referenced_symbols_for_node_exported(&host.context(), position, node, program.get_source_files());
    if entries.is_empty() {
        return Ok(Value::Null);
    }
    let mut out = Vec::new();
    for entry in &entries {
        let Some(def) = entry.definition_node() else { continue };
        let mut refs = Vec::new();
        for r in entry.references() {
            if let Some(n) = r.node() {
                refs.push(Value::String(host.node_handle(n)?));
            }
        }
        let mut o = obj();
        o.set("definition", Value::String(host.node_handle(def)?));
        if let Some(sym) = entry.definition_symbol() {
            o.set("symbol", sd.symbol_response(sym, &sd.project)?);
        }
        o.set("references", Value::Array(refs));
        out.push(o.build());
    }
    if out.is_empty() {
        // Go: `var result []ReferencedSymbolEntry` stays nil when every entry lacks a definition.
        return Ok(Value::Null);
    }
    Ok(Value::Array(out))
}

pub(crate) fn get_completions_at_position(host: &dyn CheckerHost, p: &Params) -> CheckerResult<Value> {
    let include_symbol = p.bool("includeSymbol")?;
    let mut ctx = host.context();
    if include_symbol {
        ctx = with_checker_lifetime(&ctx, CheckerLifetime::API);
    }
    let sd = snapshot_ctx(host, p)?;
    let file = p.document("file")?;
    let file_name = file.to_file_name();
    let position = p.u32("position")?;
    let trigger = match p.raw("triggerCharacter") {
        None => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(_) => return Err(CheckerError::invalid("field \"triggerCharacter\": expected a string")),
    };
    let run = |snapshot: &Arc<Snapshot>, program: &'static Program, ctx: &Context| -> CheckerResult<Result<Option<tsrs_ls::CompletionList>, tsrs_lsproto::Error>> {
        let Some(source_file) = program.get_source_file(&file_name) else {
            return Ok(Ok(None));
        };
        let ls = language_service(snapshot, program, &sd.project, "")?;
        let internal = source_file.get_position_map().utf16_to_utf8(i32::try_from(position).unwrap_or(i32::MAX));
        Ok(ls.get_completions_at_position(ctx, source_file, internal, trigger, include_symbol))
    };

    let program = sd.program()?;
    let mut prepared: Option<Arc<Snapshot>> = None;
    let mut result = run(&sd.scope.snapshot, program, &ctx)?;
    if matches!(&result, Err(e) if tsrs_ls::is_err_needs_auto_imports(e)) {
        let snapshot = host.clone_snapshot_with_auto_imports(&sd.scope.snapshot, &file_name)?;
        let outcome = program_of(&snapshot, &sd.project).and_then(|program| run(&snapshot, program, &ctx));
        prepared = Some(snapshot);
        result = match outcome {
            Ok(r) => r,
            Err(e) => {
                prepared.take().unwrap().deref();
                return Err(e);
            }
        };
    }
    let release_prepared = |prepared: Option<Arc<Snapshot>>| {
        if let Some(s) = prepared {
            s.deref();
        }
    };
    let list = match result {
        Ok(Some(list)) => list,
        Ok(None) => {
            release_prepared(prepared);
            return Ok(Value::Null);
        }
        Err(e) => {
            release_prepared(prepared);
            return Err(CheckerError::client(e.message));
        }
    };
    let mut entries = Vec::with_capacity(list.items.len());
    let mut uses_prepared_symbols = false;
    for item in &list.items {
        let ci = &item.completion_item;
        let mut o = obj();
        o.set("name", Value::String(ci.label.clone()));
        if let Some(kind) = &ci.kind {
            o.nonzero("kind", kind.0 as f64);
        }
        for (key, value) in [("sortText", &ci.sort_text), ("insertText", &ci.insert_text), ("filterText", &ci.filter_text), ("detail", &ci.detail)] {
            if let Some(v) = value {
                o.set(key, Value::String(v.clone()));
            }
        }
        if let Some(details) = &ci.label_details {
            let mut d = obj();
            if let Some(v) = &details.detail {
                d.set("detail", Value::String(v.clone()));
            }
            if let Some(v) = &details.description {
                d.set("description", Value::String(v.clone()));
            }
            o.set("labelDetails", d.build());
        }
        if let Some(symbol) = item.symbol {
            uses_prepared_symbols |= prepared.is_some();
            o.set("symbol", sd.symbol_response(symbol, &sd.project)?);
        }
        entries.push(o.build());
    }
    match prepared {
        // Symbols from the auto-import snapshot were handed out: keep it alive with the registry.
        Some(s) if uses_prepared_symbols => sd.scope.registry.retain_snapshot(s)?,
        other => release_prepared(other),
    }
    let mut o = obj();
    o.set("isIncomplete", Value::Bool(list.is_incomplete));
    o.set("entries", Value::Array(entries));
    Ok(o.build())
}

/// Go `handleGetImportAdderEdits`.
pub(crate) fn get_import_adder_edits(host: &dyn CheckerHost, p: &Params) -> CheckerResult<Value> {
    let sd = snapshot_ctx(host, p)?;
    let file = p.document("file")?;
    let file_name = file.to_file_name();
    let mut program = sd.program()?;
    let mut source_file = program.get_source_file(&file_name).ok_or_else(|| CheckerError::client(format!("source file not found: {}", file.display())))?;
    let project_id = LsProjectID(sd.project.clone());

    // Decode actions up front (bounded, explicit errors) before any snapshot work.
    let mut actions: Vec<(SymbolReference, bool)> = Vec::new();
    for (i, action) in p.array("actions")?.iter().enumerate() {
        let a = Params::new(action, "getImportAdderEdits")?;
        match a.string("kind")? {
            "importSymbol" => {
                if a.raw("symbol").is_none() {
                    return Err(CheckerError::client(format!("import adder action {i} missing symbol")));
                }
                let valid = match a.raw("isValidTypeOnlyUseSite") {
                    None => true,
                    Some(Value::Bool(b)) => *b,
                    Some(_) => return Err(CheckerError::invalid("field \"isValidTypeOnlyUseSite\": expected a boolean")),
                };
                actions.push((a.symbol_ref("symbol")?, valid));
            }
            kind => return Err(CheckerError::client(format!("unknown import adder action kind {kind:?}"))),
        }
    }

    let mut working = sd.scope.snapshot.clone();
    let mut prepared: Option<Arc<Snapshot>> = None;
    let prepared_for_file = working.auto_import_registry().is_some_and(|r| r.is_prepared_for_importing_file(source_file.file_name(), &project_id, working.user_preferences()));
    if !prepared_for_file {
        let snapshot = host.clone_snapshot_with_auto_imports(&working, &file_name)?;
        prepared = Some(snapshot.clone());
        working = snapshot;
        let resolved = program_of(&working, &sd.project).and_then(|prog| {
            let sf = prog.get_source_file(&file_name).ok_or_else(|| CheckerError::client(format!("source file not found: {}", file.display())))?;
            Ok((prog, sf))
        });
        match resolved {
            Ok((prog, sf)) => {
                program = prog;
                source_file = sf;
            }
            Err(e) => {
                working.deref();
                return Err(e);
            }
        }
    }
    let result = import_adder_edits(host, &sd, &working, program, source_file, project_id, &actions);
    if let Some(s) = prepared {
        s.deref();
    }
    result
}

fn import_adder_edits(
    host: &dyn CheckerHost,
    sd: &SnapshotCtx,
    working: &Arc<Snapshot>,
    program: &'static Program,
    source_file: P<tsrs_ast::SourceFile>,
    project_id: LsProjectID,
    actions: &[(SymbolReference, bool)],
) -> CheckerResult<Value> {
    let Some(registry) = working.auto_import_registry() else {
        return Ok(Value::Array(Vec::new()));
    };
    // Resolve symbols against the working program (Go resolves through a checker-less checkerSetup).
    let symbols: Vec<(P<Symbol>, bool)> = actions.iter().map(|(r, valid)| resolve_symbol_for_program(sd, program, r).map(|s| (s, *valid))).collect::<CheckerResult<_>>()?;
    let ctx = host.context();
    let mut checker = program.get_type_checker(&ctx);
    let preferences = working.user_preferences().clone();
    let view = tsrs_ls::autoimport::new_view(Some(registry), source_file, project_id, program, &mut checker, preferences.module_specifier_preferences());
    let format = tsrs_ls::Host::get_preferences(working.as_ref(), source_file.file_name()).format_code_settings;
    let mut adder = tsrs_ls::autoimport::new_import_adder(&ctx, program, &mut checker, source_file, view, format, working.converters(), preferences);
    for (symbol, valid) in symbols {
        adder.add_import_from_exported_symbol(symbol, valid);
    }
    if !adder.has_fixes() {
        return Ok(Value::Array(Vec::new()));
    }
    let edits = adder.edits();
    drop(adder);
    drop(checker);
    Ok(to_api_text_edits(source_file, &edits))
}

/// Go `toAPITextEdits` (+ `originalTextOffset`): LSP line/character → original-text offsets → UTF-16.
fn to_api_text_edits(source_file: P<tsrs_ast::SourceFile>, edits: &[tsrs_lsproto::TextEdit]) -> Value {
    let original = source_file.original_text();
    let line_map = tsrs_ls::lsconv::compute_lsp_line_starts(original);
    let position_map = tsrs_ast::compute_position_map(original);
    let offset = |pos: &tsrs_lsproto::Position| -> Option<i64> {
        let line = pos.line as usize;
        let start = *line_map.line_starts.get(line)? as i64;
        let off = start + pos.character as i64;
        (off >= start && off <= original.len() as i64).then_some(off)
    };
    let mut out = Vec::with_capacity(edits.len());
    for edit in edits {
        let (Some(start), Some(end)) = (offset(&edit.range.start), offset(&edit.range.end)) else {
            return Value::Null;
        };
        let mut o = obj();
        o.num("pos", position_map.utf8_to_utf16(start as i32) as f64);
        o.num("end", position_map.utf8_to_utf16(end as i32) as f64);
        o.set("newText", Value::String(edit.new_text.clone()));
        out.push(o.build());
    }
    Value::Array(out)
}
