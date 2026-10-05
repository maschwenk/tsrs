use std::sync::Arc;

use tsrs_ast::{self as ast, Diagnostic, Node, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::Checker;
use tsrs_core::context::Context;
use tsrs_core::{tspath, CompilerOptions, JsxEmit, TextRange, P};
use tsrs_diagnostics as diagnostics;
use tsrs_lsproto as lsproto;
use tsrs_scanner as scanner;

use crate::astnav;
use crate::autoimport::{self, ImportAdder, QueryKind, View};
use crate::codeactions::{is_fixable_diagnostic, CodeAction, CodeFixContext, CodeFixProvider, CombinedCodeActions};

// codeactions_importfixes.go:20
fn import_fix_error_codes() -> &'static [i32] {
    static CODES: std::sync::LazyLock<Vec<i32>> = std::sync::LazyLock::new(|| {
        vec![
            diagnostics::Cannot_find_name_0.code(),
            diagnostics::Cannot_find_name_0_Did_you_mean_1.code(),
            diagnostics::Cannot_find_name_0_Did_you_mean_the_instance_member_this_0.code(),
            diagnostics::Cannot_find_name_0_Did_you_mean_the_static_member_1_0.code(),
            diagnostics::Cannot_find_namespace_0.code(),
            diagnostics::X_0_refers_to_a_UMD_global_but_the_current_file_is_a_module_Consider_adding_an_import_instead.code(),
            diagnostics::X_0_only_refers_to_a_type_but_is_being_used_as_a_value_here.code(),
            diagnostics::No_value_exists_in_scope_for_the_shorthand_property_0_Either_declare_one_or_provide_an_initializer.code(),
            diagnostics::X_0_cannot_be_used_as_a_value_because_it_was_imported_using_import_type.code(),
            diagnostics::Cannot_find_name_0_Do_you_need_to_install_type_definitions_for_jQuery_Try_npm_i_save_dev_types_Slashjquery.code(),
            diagnostics::Cannot_find_name_0_Do_you_need_to_change_your_target_library_Try_changing_the_lib_compiler_option_to_1_or_later.code(),
            diagnostics::Cannot_find_name_0_Do_you_need_to_change_your_target_library_Try_changing_the_lib_compiler_option_to_include_dom.code(),
            diagnostics::Cannot_find_name_0_Do_you_need_to_install_type_definitions_for_a_test_runner_Try_npm_i_save_dev_types_Slashjest_or_npm_i_save_dev_types_Slashmocha_and_then_add_jest_or_mocha_to_the_types_field_in_your_tsconfig.code(),
            diagnostics::Cannot_find_name_0_Did_you_mean_to_write_this_in_an_async_function.code(),
            diagnostics::Cannot_find_name_0_Do_you_need_to_install_type_definitions_for_jQuery_Try_npm_i_save_dev_types_Slashjquery_and_then_add_jquery_to_the_types_field_in_your_tsconfig.code(),
            diagnostics::Cannot_find_name_0_Do_you_need_to_install_type_definitions_for_a_test_runner_Try_npm_i_save_dev_types_Slashjest_or_npm_i_save_dev_types_Slashmocha.code(),
            diagnostics::Cannot_find_name_0_Do_you_need_to_install_type_definitions_for_node_Try_npm_i_save_dev_types_Slashnode.code(),
            diagnostics::Cannot_find_name_0_Do_you_need_to_install_type_definitions_for_node_Try_npm_i_save_dev_types_Slashnode_and_then_add_node_to_the_types_field_in_your_tsconfig.code(),
            diagnostics::Cannot_find_namespace_0_Did_you_mean_1.code(),
            diagnostics::Cannot_extend_an_interface_0_Did_you_mean_implements.code(),
            diagnostics::This_JSX_tag_requires_0_to_be_in_scope_but_it_could_not_be_found.code(),
        ]
    });
    &CODES
}

// codeactions_importfixes.go:44
const IMPORT_FIX_ID: &str = "fixMissingImport";

// codeactions_importfixes.go:48
// ImportFixProvider is the CodeFixProvider for import-related fixes
pub(crate) static IMPORT_FIX_PROVIDER: std::sync::LazyLock<CodeFixProvider> = std::sync::LazyLock::new(|| CodeFixProvider {
    error_codes: import_fix_error_codes(),
    get_code_actions: get_import_code_actions,
    fix_ids: &[IMPORT_FIX_ID],
    get_all_code_actions: Some(get_all_import_code_actions),
});

// codeactions_importfixes.go:55
struct fixInfo {
    fix: Arc<autoimport::Fix>,
    symbol_name: String,
    error_identifier_text: String,
    is_jsx_namespace_fix: bool,
}

// codeactions_importfixes.go:62
fn get_import_code_actions(ctx: &Context, fix_context: &CodeFixContext) -> Result<Vec<CodeAction>, lsproto::Error> {
    let mut ch = fix_context.program.get_type_checker(ctx);

    let info = get_fix_infos(&mut ch, fix_context, fix_context.error_code, fix_context.span.pos())?;
    if info.is_empty() {
        return Ok(Vec::new());
    }

    let mut actions = Vec::new();
    for fix_info in &info {
        let (edits, description, ok) = fix_info.fix.edits(
            ctx,
            fix_context.source_file,
            fix_context.program.options(),
            fix_context.ls.format_options(),
            &fix_context.ls.converters,
            fix_context.ls.user_preferences(),
        );

        if ok {
            actions.push(CodeAction {
                description,
                changes: edits,
                fix_id: IMPORT_FIX_ID.to_string(),
                fix_all_description: diagnostics::Add_all_missing_imports.localize(&[]),
            });
        }
    }
    Ok(actions)
}

// codeactions_importfixes.go:96
fn get_all_import_code_actions(ctx: &Context, fix_context: &CodeFixContext) -> Result<Option<CombinedCodeActions>, lsproto::Error> {
    if tspath::is_dynamic_file_name(fix_context.source_file.file_name()) {
        return Ok(None);
    }

    let all_diagnostics = fix_context.program.get_semantic_diagnostics(ctx, Some(fix_context.source_file));

    let mut import_diags: Vec<P<Diagnostic>> = Vec::new();
    for diag in all_diagnostics {
        if is_fixable_diagnostic(diag, import_fix_error_codes()) {
            import_diags.push(diag);
        }
    }

    if import_diags.is_empty() {
        return Ok(None);
    }

    let mut ch = fix_context.program.get_type_checker(ctx);

    // Go falls back to the current view when the prepared view is nil; a prepared view is never nil.
    let view = fix_context.ls.get_prepared_auto_import_view(fix_context.source_file, &mut ch)?;

    let mut import_adder = autoimport::new_import_adder(
        ctx,
        fix_context.program,
        &mut ch,
        fix_context.source_file,
        view,
        fix_context.ls.format_options(),
        Arc::clone(&fix_context.ls.converters),
        fix_context.ls.user_preferences().clone(),
    );

    for diag in &import_diags {
        add_import_from_diagnostic(&mut ch, &mut *import_adder, *diag, fix_context)?;
    }

    if !import_adder.has_fixes() {
        return Ok(None);
    }

    Ok(Some(CombinedCodeActions { description: diagnostics::Add_all_missing_imports.localize(&[]), changes: import_adder.edits() }))
}

// codeactions_importfixes.go:150
// addImportFromDiagnostic finds the best import fix for a diagnostic and adds it to the adder.
fn add_import_from_diagnostic(
    ch: &mut Checker,
    import_adder: &mut dyn ImportAdder,
    diag: P<Diagnostic>,
    fix_context: &CodeFixContext,
) -> Result<(), lsproto::Error> {
    let diag_fix_context = CodeFixContext {
        source_file: fix_context.source_file,
        span: TextRange::new(diag.pos(), diag.end()),
        error_code: diag.code(),
        program: fix_context.program,
        ls: fix_context.ls,
        diagnostic: None,
        params: None,
    };

    let infos = get_fix_infos(ch, &diag_fix_context, diag.code(), diag.pos())?;
    if !infos.is_empty() {
        import_adder.add_import_fix(&infos[0].fix);
    }
    Ok(())
}

// codeactions_importfixes.go:169
fn get_fix_infos(ch: &mut Checker, fix_context: &CodeFixContext, error_code: i32, pos: i32) -> Result<Vec<fixInfo>, lsproto::Error> {
    // Can't compute import fixes for dynamic/untitled files since they don't have real file paths
    if tspath::is_dynamic_file_name(fix_context.source_file.file_name()) {
        return Ok(Vec::new());
    }

    let symbol_token = astnav::get_token_at_position(fix_context.source_file, pos);
    if error_code != diagnostics::X_0_refers_to_a_UMD_global_but_the_current_file_is_a_module_Consider_adding_an_import_instead.code()
        && !ast::is_identifier(symbol_token)
    {
        return Ok(Vec::new());
    }

    let view: Option<View>;
    let mut info: Vec<fixInfo>;

    if error_code == diagnostics::X_0_refers_to_a_UMD_global_but_the_current_file_is_a_module_Consider_adding_an_import_instead.code() {
        let v = fix_context.ls.get_current_auto_import_view(fix_context.source_file, ch);
        info = get_fixes_info_for_umd_import(symbol_token, &v, ch);
        view = Some(v);
    } else if error_code == diagnostics::X_0_cannot_be_used_as_a_value_because_it_was_imported_using_import_type.code() {
        let compiler_options = fix_context.program.options();
        let symbol_names = get_symbol_names_to_import(fix_context.source_file, ch, symbol_token, &compiler_options);

        let mut all_type_only_fixes: Vec<fixInfo> = Vec::new();
        for sn in &symbol_names {
            if !sn.is_type_only {
                continue;
            }
            if let Some(fix) = get_type_only_promotion_fix(fix_context.source_file, symbol_token, &sn.name, ch) {
                all_type_only_fixes.push(fixInfo {
                    fix: Arc::new(fix),
                    symbol_name: sn.name.clone(),
                    error_identifier_text: symbol_token.text().to_string(),
                    is_jsx_namespace_fix: false,
                });
            }
        }

        // For JSX opening tags, there can be separate type-only errors for both the tag name
        // identifier and the JSX namespace identifier. When both produce valid fixes, we
        // disambiguate using the diagnostic message, which quotes the symbol name in single
        // quotes (e.g., "'React' cannot be used as a value..."). If filtering yields nothing
        // (e.g., due to localization), fall back to returning all candidates.
        let mut diagnostic_message = String::new();
        if let Some(d) = fix_context.diagnostic {
            diagnostic_message = d.message.as_string();
        }
        if all_type_only_fixes.len() > 1 && !diagnostic_message.is_empty() {
            let (matching, rest): (Vec<fixInfo>, Vec<fixInfo>) =
                all_type_only_fixes.into_iter().partition(|fi| diagnostic_message.contains(&format!("'{}'", fi.symbol_name)));
            info = matching;
            all_type_only_fixes = rest;
            if info.is_empty() {
                info = all_type_only_fixes;
            }
            return Ok(info);
        }
        return Ok(all_type_only_fixes);
    } else {
        let v = fix_context.ls.get_prepared_auto_import_view(fix_context.source_file, ch)?;
        info = get_fixes_info_for_non_umd_import(fix_context, symbol_token, &v, ch);
        view = Some(v);
    }

    // Sort fixes by preference
    let view = view.unwrap_or_else(|| fix_context.ls.get_current_auto_import_view(fix_context.source_file, ch));
    Ok(sort_fix_info(info, &view))
}

// codeactions_importfixes.go:240
fn get_fixes_info_for_umd_import(token: P<Node>, view: &View, ch: &mut Checker) -> Vec<fixInfo> {
    let Some(umd_symbol) = get_umd_symbol(token, ch) else {
        return Vec::new();
    };

    // Go passes the (possibly nil) export on; GetFixes dereferences it.
    let export = autoimport::symbol_to_export(umd_symbol, ch).expect("nil export");
    let is_valid_type_only_use_site = ast::is_valid_type_only_alias_use_site(token);

    let mut result = Vec::new();
    for fix in view.get_fixes(&export, false, is_valid_type_only_use_site, None) {
        let mut error_identifier_text = String::new();
        if ast::is_identifier(token) {
            error_identifier_text = token.text().to_string();
        }
        result.push(fixInfo { fix, symbol_name: umd_symbol.name().to_string(), error_identifier_text, is_jsx_namespace_fix: false });
    }
    result
}

// codeactions_importfixes.go:264
fn get_umd_symbol(token: P<Node>, ch: &mut Checker) -> Option<P<Symbol>> {
    // try the identifier to see if it is the umd symbol
    let mut umd_symbol: Option<P<Symbol>> = None;
    if ast::is_identifier(token) {
        umd_symbol = Some(ch.get_resolved_symbol_exported(token));
    }
    if is_umd_export_symbol(umd_symbol) {
        return umd_symbol;
    }

    // The error wasn't for the symbolAtLocation, it was for the JSX tag itself, which needs access to e.g. `React`.
    let parent = token.parent().unwrap();
    if (ast::is_jsx_opening_like_element(parent) && parent.tag_name() == token) || ast::is_jsx_opening_fragment(parent) {
        let location = if ast::is_jsx_opening_like_element(parent) { token } else { parent };
        let jsx_namespace = ch.get_jsx_namespace_exported(parent);
        let parent_symbol = ch.resolve_name_exported(&jsx_namespace, location, SymbolFlags::Value, false /* excludeGlobals */);
        if is_umd_export_symbol(parent_symbol) {
            return parent_symbol;
        }
    }
    None
}

// codeactions_importfixes.go:291
fn is_umd_export_symbol(symbol: Option<P<Symbol>>) -> bool {
    symbol.is_some_and(|s| !s.declarations().is_empty() && ast::is_namespace_export_declaration(s.declarations()[0]))
}

// codeactions_importfixes.go:297
fn get_fixes_info_for_non_umd_import(fix_context: &CodeFixContext, symbol_token: P<Node>, view: &View, ch: &mut Checker) -> Vec<fixInfo> {
    let compiler_options = fix_context.program.options();

    let is_valid_type_only_use_site = ast::is_valid_type_only_alias_use_site(symbol_token);
    let symbol_names = get_symbol_names_to_import(fix_context.source_file, ch, symbol_token, &compiler_options);
    let mut all_info: Vec<fixInfo> = Vec::new();

    // Compute usage position for JSDoc import type fixes
    let (usage_position, fidelity) =
        fix_context.ls.converters.to_lsp_position(&fix_context.source_file, scanner::get_token_pos_of_node(symbol_token, fix_context.source_file, false));
    if !fidelity.is_exact() {
        return Vec::new();
    }

    for sn in &symbol_names {
        // Type-only imports are handled by the promotion code path, not the auto-import path.
        if sn.is_type_only {
            continue;
        }

        let symbol_name = &sn.name;
        // "default" is a keyword and not a legal identifier for the import
        if symbol_name == "default" {
            continue;
        }

        let is_jsx_tag_name = symbol_name == symbol_token.text() && ast::is_jsx_tag_name(symbol_token);
        let mut query_kind = QueryKind::ExactMatch;
        if is_jsx_tag_name {
            query_kind = QueryKind::CaseInsensitiveMatch;
        }

        let exports = view.search(symbol_name, query_kind);
        for export in &exports {
            if is_jsx_tag_name && !(export.name() == symbol_name || export.is_renameable()) {
                continue;
            }

            let fixes = view.get_fixes(export, is_jsx_tag_name, is_valid_type_only_use_site, Some(usage_position));
            for fix in fixes {
                all_info.push(fixInfo {
                    fix,
                    symbol_name: symbol_name.clone(),
                    error_identifier_text: String::new(),
                    is_jsx_namespace_fix: symbol_name != symbol_token.text(),
                });
            }
        }
    }

    all_info
}

// codeactions_importfixes.go:351
fn get_type_only_promotion_fix(source_file: P<SourceFile>, symbol_token: P<Node>, symbol_name: &str, ch: &mut Checker) -> Option<autoimport::Fix> {
    // Get the symbol at the token location
    let symbol = ch.resolve_name_exported(symbol_name, symbol_token, SymbolFlags::Value, true /* excludeGlobals */)?;

    // Get the type-only alias declaration
    let type_only_alias_declaration = ch.get_type_only_alias_declaration(symbol)?;
    if ast::get_source_file_of_node(type_only_alias_declaration) != Some(source_file) {
        return None;
    }

    Some(autoimport::Fix {
        auto_import_fix: lsproto::AutoImportFix { kind: lsproto::AutoImportFixKind::PromoteTypeOnly, ..Default::default() },
        type_only_alias_declaration: Some(type_only_alias_declaration),
        ..Default::default()
    })
}

// codeactions_importfixes.go:372
struct symbolNameInfo {
    name: String,
    is_type_only: bool, // whether the symbol currently resolves to a type-only import
}

// codeactions_importfixes.go:377
fn get_symbol_names_to_import(source_file: P<SourceFile>, ch: &mut Checker, symbol_token: P<Node>, compiler_options: &CompilerOptions) -> Vec<symbolNameInfo> {
    let parent = symbol_token.parent().unwrap();
    if (ast::is_jsx_opening_like_element(parent) || ast::is_jsx_closing_element(parent))
        && parent.tag_name() == symbol_token
        && jsx_mode_needs_explicit_import(compiler_options.jsx)
    {
        let jsx_namespace = ch.get_jsx_namespace_exported(source_file.as_node());
        if needs_jsx_namespace_fix(&jsx_namespace, symbol_token, ch) {
            let mut result = Vec::new();
            if !scanner::is_intrinsic_jsx_name(symbol_token.text()) {
                let comp_symbol = ch.resolve_name_exported(symbol_token.text(), symbol_token, SymbolFlags::Value, false /* excludeGlobals */);
                match comp_symbol {
                    None => result.push(symbolNameInfo { name: symbol_token.text().to_string(), is_type_only: false }),
                    Some(comp_symbol) => {
                        if ch.get_type_only_alias_declaration(comp_symbol).is_some() {
                            result.push(symbolNameInfo { name: symbol_token.text().to_string(), is_type_only: true });
                        }
                    }
                }
            }
            let mut ns_is_type_only = false;
            if let Some(ns_symbol) = ch.resolve_name_exported(&jsx_namespace, symbol_token, SymbolFlags::Value, true /* excludeGlobals */) {
                ns_is_type_only = ch.get_type_only_alias_declaration(ns_symbol).is_some();
            }
            result.push(symbolNameInfo { name: jsx_namespace, is_type_only: ns_is_type_only });
            return result;
        }
    }
    let mut token_is_type_only = false;
    if let Some(sym) = ch.resolve_name_exported(symbol_token.text(), symbol_token, SymbolFlags::Value, true /* excludeGlobals */) {
        token_is_type_only = ch.get_type_only_alias_declaration(sym).is_some();
    }
    vec![symbolNameInfo { name: symbol_token.text().to_string(), is_type_only: token_is_type_only }]
}

// codeactions_importfixes.go:411
fn needs_jsx_namespace_fix(jsx_namespace: &str, symbol_token: P<Node>, ch: &mut Checker) -> bool {
    if scanner::is_intrinsic_jsx_name(symbol_token.text()) {
        return true;
    }
    let Some(namespace_symbol) = ch.resolve_name_exported(jsx_namespace, symbol_token, SymbolFlags::Value, true /* excludeGlobals */) else {
        return true;
    };
    if namespace_symbol.declarations().iter().any(|&d| ast::is_type_only_import_or_export_declaration(d)) {
        return !namespace_symbol.flags().intersects(SymbolFlags::Value);
    }
    false
}

// codeactions_importfixes.go:425
fn jsx_mode_needs_explicit_import(jsx: JsxEmit) -> bool {
    jsx == JsxEmit::React || jsx == JsxEmit::ReactNative
}

// codeactions_importfixes.go:429
fn sort_fix_info(fixes: Vec<fixInfo>, view: &View) -> Vec<fixInfo> {
    if fixes.is_empty() {
        return fixes;
    }

    // Create a copy to avoid modifying the original
    let mut sorted = fixes;

    // Sort by:
    // 1. JSX namespace fixes last
    // 2. Fix comparison using view.CompareFixes
    tsrs_core::goslices::sort_func(&mut sorted, |a, b| {
        // JSX namespace fixes should come last
        let cmp = tsrs_core::compare_booleans(a.is_jsx_namespace_fix, b.is_jsx_namespace_fix);
        if cmp != 0 {
            return cmp;
        }
        view.compare_fixes_for_sorting(&a.fix, &b.fix)
    });

    sorted
}
