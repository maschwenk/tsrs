use tsrs_ast::{Diagnostic, DiagnosticExt as _, SourceFile};
use tsrs_compiler::Program;
use tsrs_core::collections::OrderedMap;
use tsrs_core::context::Context;
use tsrs_core::{TextRange, P};
use tsrs_diagnostics as diagnostics;
use tsrs_lsproto as lsproto;

use crate::diagnostics::get_all_diagnostics;
use crate::languageservice::LanguageService;
use crate::lsconv;
use crate::spanmap::Feature;

// codeactions.go:21
// CodeFixProvider represents a provider for a specific type of code fix
pub struct CodeFixProvider {
    pub error_codes: &'static [i32],
    pub get_code_actions: fn(ctx: &Context, fix_context: &CodeFixContext) -> Result<Vec<CodeAction>, lsproto::Error>,
    pub fix_ids: &'static [&'static str],
    pub get_all_code_actions: Option<fn(ctx: &Context, fix_context: &CodeFixContext) -> Result<Option<CombinedCodeActions>, lsproto::Error>>,
}

// codeactions.go:29
// CodeFixContext contains the context needed to generate code fixes
pub struct CodeFixContext<'a> {
    pub source_file: P<SourceFile>,
    pub span: TextRange,
    pub error_code: i32,
    pub program: &'static Program,
    pub ls: &'a LanguageService,
    pub diagnostic: Option<&'a lsproto::Diagnostic>,
    pub params: Option<&'a lsproto::CodeActionParams>,
}

// codeactions.go:40
// CodeAction represents a single code action fix
#[derive(Clone, Debug, Default)]
pub struct CodeAction {
    pub description: String,
    pub changes: Vec<lsproto::TextEdit>,
    pub fix_id: String,
    pub fix_all_description: String,
}

impl CodeAction {
    // codeactions.go:49
    // Compare defines a total ordering for CodeAction values, comparing description
    // then text edits lexicographically. Used with slices.BinarySearchFunc.
    pub fn compare(&self, b: &CodeAction) -> i32 {
        let c = go_compare(&self.description, &b.description);
        if c != 0 {
            return c;
        }
        let c = go_compare(&self.changes.len(), &b.changes.len());
        if c != 0 {
            return c;
        }
        for (i, edit) in self.changes.iter().enumerate() {
            let c = edit.compare(&b.changes[i]);
            if c != 0 {
                return c;
            }
        }
        0
    }
}

fn go_compare<T: Ord + ?Sized>(a: &T, b: &T) -> i32 {
    match a.cmp(b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

// codeactions.go:66
// CombinedCodeActions represents combined code actions for fix-all scenarios
#[derive(Clone, Debug, Default)]
pub struct CombinedCodeActions {
    pub description: String,
    pub changes: Vec<lsproto::TextEdit>,
}

// codeactions.go:72
// codeFixProviders is the list of all registered code fix providers
fn code_fix_providers() -> &'static [&'static CodeFixProvider] {
    static PROVIDERS: &[&CodeFixProvider] = &[
        // Add more code fix providers here as they are implemented
    ];
    PROVIDERS
}

impl LanguageService {
    // codeactions.go:80
    // ProvideCodeActions returns code actions for the given range and context
    pub fn provide_code_actions(&self, ctx: &Context, params: &lsproto::CodeActionParams) -> Result<lsproto::CodeActionResponse, lsproto::Error> {
        let (program, file) = self.get_program_and_file(&params.text_document.uri);

        let mut actions: Vec<lsproto::CommandOrCodeAction> = Vec::new();

        if let Some(only) = &params.context.only {
            for &kind in only {
                let matching_kinds = get_organize_imports_actions_for_kind(kind);
                for matching_kind in matching_kinds {
                    let organize_action = self.create_organize_imports_action(ctx, program, file, matching_kind);
                    actions.push(organize_action);
                }

                if is_fix_all_kind(kind) {
                    let fix_all_action = self.create_fix_all_action(ctx, program, file, &params.text_document.uri)?;
                    if let Some(fix_all_action) = fix_all_action {
                        actions.push(fix_all_action);
                    }
                }
            }
        }

        // Go: `params.Context.Diagnostics != nil`; a decoded required array is never nil.
        if wants_quick_fixes(params.context.only.as_deref()) {
            // Go map (random iteration order).
            let mut fix_id_seen: OrderedMap<String, &'static CodeFixProvider> = OrderedMap::default();

            let mut seen: Vec<CodeAction> = Vec::new(); // sorted for binary search dedup, dedup across all diagnostics and providers so if multiple diags produce the same codefix, only one is returned

            for diag in &params.context.diagnostics {
                let Some(error_code) = diag.code.as_ref().and_then(|c| c.integer) else {
                    continue;
                };

                for &provider in code_fix_providers() {
                    if !code_fix_provider_matches_lsp_diagnostic(provider, diag) {
                        continue;
                    }

                    for mapped in self.converters.from_lsp_range_for_source_file(file, diag.range, Feature::CodeActions) {
                        let fix_context = CodeFixContext {
                            source_file: mapped.script,
                            span: mapped.span,
                            error_code,
                            program,
                            ls: self,
                            diagnostic: Some(diag),
                            params: Some(params),
                        };

                        let provider_actions = (provider.get_code_actions)(ctx, &fix_context)?;
                        for action in provider_actions {
                            let i = match seen.binary_search_by(|probe| probe.compare(&action).cmp(&0)) {
                                Ok(_) => continue,
                                Err(i) => i,
                            };
                            actions.push(convert_to_lsp_code_action(&action, diag, &params.text_document.uri));
                            if !action.fix_id.is_empty() {
                                fix_id_seen.insert(action.fix_id.clone(), provider);
                            }
                            seen.insert(i, action);
                        }
                    }
                }
            }

            let fix_all_actions = self.get_fix_all_quick_fixes(ctx, program, file, &params.text_document.uri, &fix_id_seen)?;
            actions.extend(fix_all_actions);
        }

        Ok(lsproto::CommandOrCodeActionArrayOrNull { command_or_code_action_array: Some(actions), ..Default::default() })
    }

    // codeactions.go:154
    // getFixAllQuickFixes returns per-provider "Fix all in file" quickfix entries for providers
    // that matched at least 2 diagnostics in the full file.
    fn get_fix_all_quick_fixes(
        &self,
        ctx: &Context,
        program: &'static Program,
        file: P<SourceFile>,
        uri: &lsproto::DocumentUri,
        fix_id_seen: &OrderedMap<String, &'static CodeFixProvider>,
    ) -> Result<Vec<lsproto::CommandOrCodeAction>, lsproto::Error> {
        let mut actions = Vec::new();

        // Deduplicate providers; multiple fixIds may map to the same provider.
        let mut seen: Vec<&'static CodeFixProvider> = Vec::new();
        for &provider in fix_id_seen.values() {
            if seen.iter().any(|&p| std::ptr::eq(p, provider)) {
                continue;
            }
            seen.push(provider);

            let Some(get_all_code_actions) = provider.get_all_code_actions else {
                continue;
            };

            if !has_multiple_fixable_diagnostics(ctx, program, file, provider.error_codes) {
                continue;
            }

            let fix_context = CodeFixContext { source_file: file, span: TextRange::default(), error_code: 0, program, ls: self, diagnostic: None, params: None };
            let combined = get_all_code_actions(ctx, &fix_context)?;
            if let Some(combined) = combined {
                if !combined.changes.is_empty() {
                    let kind = lsproto::CodeActionKind::QuickFix;
                    let mut changes = OrderedMap::default();
                    changes.insert(uri.clone(), combined.changes);
                    actions.push(lsproto::CommandOrCodeAction {
                        code_action: Some(lsproto::CodeAction {
                            title: combined.description,
                            kind: Some(kind),
                            edit: Some(lsproto::WorkspaceEdit { changes: Some(changes), ..Default::default() }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
            }
        }

        Ok(actions)
    }

    // codeactions.go:250
    // createFixAllAction creates a source.fixAll code action that applies all auto-fixable
    // code fixes across the file.
    fn create_fix_all_action(
        &self,
        ctx: &Context,
        program: &'static Program,
        file: P<SourceFile>,
        uri: &lsproto::DocumentUri,
    ) -> Result<Option<lsproto::CommandOrCodeAction>, lsproto::Error> {
        let kind = lsproto::CodeActionKind::SourceFixAllTs;
        let mut lsp_changes: OrderedMap<lsproto::DocumentUri, Vec<lsproto::TextEdit>> = OrderedMap::default();

        for &provider in code_fix_providers() {
            let Some(get_all_code_actions) = provider.get_all_code_actions else {
                continue;
            };

            let fix_context = CodeFixContext { source_file: file, span: TextRange::default(), error_code: 0, program, ls: self, diagnostic: None, params: None };

            let combined = get_all_code_actions(ctx, &fix_context)?;
            if let Some(combined) = combined {
                if !combined.changes.is_empty() {
                    lsp_changes.entry(uri.clone()).or_default().extend(combined.changes);
                }
            }
        }

        if lsp_changes.is_empty() {
            return Ok(None);
        }

        Ok(Some(lsproto::CommandOrCodeAction {
            code_action: Some(lsproto::CodeAction {
                title: diagnostics::Fix_All.localize(&[]),
                kind: Some(kind),
                edit: Some(lsproto::WorkspaceEdit { changes: Some(lsp_changes), ..Default::default() }),
                ..Default::default()
            }),
            ..Default::default()
        }))
    }

    // codeactions.go:340
    // createOrganizeImportsAction creates the organize imports code action
    fn create_organize_imports_action(
        &self,
        ctx: &Context,
        program: &'static Program,
        file: P<SourceFile>,
        kind: lsproto::CodeActionKind,
    ) -> lsproto::CommandOrCodeAction {
        let title = get_organize_imports_action_title(kind);
        let changes = self.organize_imports(ctx, file, program, kind);
        if changes.is_empty() {
            return lsproto::CommandOrCodeAction {
                code_action: Some(lsproto::CodeAction {
                    title,
                    kind: Some(kind),
                    edit: Some(lsproto::WorkspaceEdit { changes: Some(OrderedMap::default()), ..Default::default() }),
                    ..Default::default()
                }),
                ..Default::default()
            };
        }

        let mut lsp_changes: OrderedMap<lsproto::DocumentUri, Vec<lsproto::TextEdit>> = OrderedMap::default();
        for (file_name, edits) in changes {
            let file_uri = lsconv::file_name_to_document_uri(&file_name);
            lsp_changes.insert(file_uri, edits);
        }

        lsproto::CommandOrCodeAction {
            code_action: Some(lsproto::CodeAction {
                title,
                kind: Some(kind),
                edit: Some(lsproto::WorkspaceEdit { changes: Some(lsp_changes), ..Default::default() }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

// codeactions.go:200
// hasMultipleFixableDiagnostics returns true if the file has at least 2 diagnostics
// matching the given error codes. Checks all diagnostic sources (semantic,
// syntactic, suggestion, declaration) to match ProvideDiagnostics.
fn has_multiple_fixable_diagnostics(ctx: &Context, program: &'static Program, file: P<SourceFile>, error_codes: &[i32]) -> bool {
    let all_diags = get_all_diagnostics(ctx, program, file);
    let mut count = 0;
    for d in all_diags {
        if is_fixable_diagnostic(d, error_codes) {
            count += 1;
            if count >= 2 {
                return true;
            }
        }
    }
    false
}

// codeactions.go:213
fn code_fix_provider_matches_lsp_diagnostic(provider: &CodeFixProvider, diagnostic: &lsproto::Diagnostic) -> bool {
    if let Some(source) = &diagnostic.source {
        if source != "ts" {
            return false;
        }
    }
    diagnostic.code.as_ref().and_then(|c| c.integer).is_some_and(|code| contains_error_code(provider.error_codes, code))
}

// codeactions.go:220
fn is_fixable_diagnostic(diagnostic: P<Diagnostic>, error_codes: &[i32]) -> bool {
    diagnostic.source().is_empty() && contains_error_code(error_codes, diagnostic.code())
}

// codeactions.go:225
// isFixAllKind returns true if the requested kind matches source.fixAll
fn is_fix_all_kind(kind: lsproto::CodeActionKind) -> bool {
    kind.contains(lsproto::CodeActionKind::SourceFixAllTs)
}

// codeactions.go:231
// wantsQuickFixes returns true if the Only filter is nil/empty (meaning all kinds are wanted)
// or explicitly includes the quickfix kind.
fn wants_quick_fixes(only: Option<&[lsproto::CodeActionKind]>) -> bool {
    let Some(only) = only else {
        return true;
    };
    if only.is_empty() {
        return true;
    }
    for &kind in only {
        if kind.contains(lsproto::CodeActionKind::QuickFix) {
            return true;
        }
    }
    false
}

// codeactions.go:299
// getOrganizeImportsActionTitle returns the appropriate title for the given organize imports kind
fn get_organize_imports_action_title(kind: lsproto::CodeActionKind) -> String {
    match kind {
        lsproto::CodeActionKind::SourceRemoveUnusedImportsTs => diagnostics::Remove_Unused_Imports.localize(&[]),
        lsproto::CodeActionKind::SourceSortImportsTs => diagnostics::Sort_Imports.localize(&[]),
        _ => diagnostics::Organize_Imports.localize(&[]),
    }
}

// codeactions.go:313
// getOrganizeImportsActionsForKind returns the organize imports code action kinds that should be
// returned for the given requested kind.
fn get_organize_imports_actions_for_kind(requested_kind: lsproto::CodeActionKind) -> Vec<lsproto::CodeActionKind> {
    let organize_imports_kinds = [
        lsproto::CodeActionKind::SourceOrganizeImportsTs,
        lsproto::CodeActionKind::SourceRemoveUnusedImportsTs,
        lsproto::CodeActionKind::SourceSortImportsTs,
    ];

    let mut result = Vec::new();
    for organize_kind in organize_imports_kinds {
        if requested_kind.contains(organize_kind) {
            result.push(organize_kind);
        }
    }

    if result.contains(&requested_kind) {
        return vec![requested_kind];
    }

    result
}

// codeactions.go:378
// containsErrorCode checks if the error code is in the list
pub(crate) fn contains_error_code(codes: &[i32], code: i32) -> bool {
    codes.contains(&code)
}

// codeactions.go:383
// convertToLSPCodeAction converts an internal CodeAction to an LSP CodeAction
fn convert_to_lsp_code_action(action: &CodeAction, diag: &lsproto::Diagnostic, uri: &lsproto::DocumentUri) -> lsproto::CommandOrCodeAction {
    let kind = lsproto::CodeActionKind::QuickFix;
    let mut changes = OrderedMap::default();
    changes.insert(uri.clone(), action.changes.clone());
    let diagnostics = vec![diag.clone()];

    lsproto::CommandOrCodeAction {
        code_action: Some(lsproto::CodeAction {
            title: action.description.clone(),
            kind: Some(kind),
            edit: Some(lsproto::WorkspaceEdit { changes: Some(changes), ..Default::default() }),
            diagnostics: Some(diagnostics),
            ..Default::default()
        }),
        ..Default::default()
    }
}
