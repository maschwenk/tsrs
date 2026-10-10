use std::sync::Arc;
use tsrs_ast::{self as ast, Node, SourceFile, Symbol, TokenFlags};
use tsrs_checker::Checker;
use tsrs_compiler::Program;
use tsrs_core::context::Context;
use tsrs_core::tspath::{self, ComparePathsOptions};
use tsrs_core::{alloc_str, TextRange, P};
use tsrs_lsproto as lsproto;
use tsrs_modulespecifiers::{self as modulespecifiers, ModuleSpecifierOptions};
use tsrs_scanner as scanner;
use tsrs_tsoptions::{self as tsoptions, CommandLineOptionKind};

use crate::change;
use crate::languageservice::LanguageService;
use crate::lsconv::{self, Converters};

// file_rename.go:22
type pathUpdater<'a> = Box<dyn Fn(&str) -> Option<String> + 'a>;

// file_rename.go:24
struct toImport {
    new_file_name: String,
    updated: bool,
}

// file_rename.go:29
struct movedFile {
    source_file: P<SourceFile>,
    new_file_name: String,
}

impl LanguageService {
    // file_rename.go:34
    pub fn get_edits_for_file_rename(
        &self,
        ctx: &Context,
        old_uri: &lsproto::DocumentUri,
        new_uri: &lsproto::DocumentUri,
    ) -> Vec<lsproto::TextDocumentEditOrCreateFileOrRenameFileOrDeleteFile> {
        let program = self.get_program();
        let old_path = old_uri.file_name();
        let new_path = new_uri.file_name();

        let old_to_new = self.create_path_updater(&old_path, &new_path);

        let mut change_tracker = change::new_tracker(ctx, &program.options(), self.format_options(), Arc::clone(&self.converters));
        self.update_tsconfig_files(program, &mut change_tracker, &old_to_new, &old_path, &new_path);
        self.update_imports_for_file_rename(program, &mut change_tracker, &old_to_new);

        let mut document_changes: Vec<lsproto::TextDocumentEditOrCreateFileOrRenameFileOrDeleteFile> = Vec::new();

        // When renaming e.g. `foo.d.css.ts` -> `bar.d.css.ts`, also rename `foo.css` -> `bar.css` if it exists.
        if tspath::is_declaration_file_name(&old_path) && tspath::is_declaration_file_name(&new_path) {
            let dts_ext = tspath::get_declaration_file_extension(&old_path);
            let original_extensions = tspath::get_possible_original_input_extension_for_extension(&dts_ext);
            for ext in &original_extensions {
                let old_original_path = tspath::change_full_extension(&old_path, ext);
                if self.host.file_exists(&old_original_path) {
                    let new_dts_ext = tspath::get_declaration_file_extension(&old_path);
                    let new_original_extensions = tspath::get_possible_original_input_extension_for_extension(&new_dts_ext);
                    if new_original_extensions.contains(ext) {
                        let new_original_path = tspath::change_full_extension(&new_path, ext);
                        document_changes.push(lsproto::TextDocumentEditOrCreateFileOrRenameFileOrDeleteFile {
                            rename_file: Some(lsproto::RenameFile {
                                old_uri: lsconv::file_name_to_document_uri(&old_original_path),
                                new_uri: lsconv::file_name_to_document_uri(&new_original_path),
                                ..Default::default()
                            }),
                            ..Default::default()
                        });
                    }
                }
            }
        }

        let (changes, _) = change_tracker.get_changes();
        for (file_name, edits) in changes {
            let uri = lsconv::file_name_to_document_uri(&file_name);
            let lsp_edits: Vec<lsproto::TextEditOrAnnotatedTextEditOrSnippetTextEdit> =
                edits.into_iter().map(|edit| lsproto::TextEditOrAnnotatedTextEditOrSnippetTextEdit { text_edit: Some(edit), ..Default::default() }).collect();
            document_changes.push(lsproto::TextDocumentEditOrCreateFileOrRenameFileOrDeleteFile {
                text_document_edit: Some(lsproto::TextDocumentEdit {
                    text_document: lsproto::OptionalVersionedTextDocumentIdentifier { uri, ..Default::default() },
                    edits: lsp_edits,
                }),
                ..Default::default()
            });
        }

        document_changes
    }

    // file_rename.go:87
    fn create_path_updater<'a>(&'a self, old_path: &str, new_path: &str) -> pathUpdater<'a> {
        let compare_options = ComparePathsOptions { use_case_sensitive_file_names: self.use_case_sensitive_file_names(), ..Default::default() };
        let trimmed_old_path = tspath::remove_trailing_directory_separator(old_path).to_string();
        let old_path = old_path.to_string();
        let new_path = new_path.to_string();
        Box::new(move |path: &str| -> Option<String> {
            if tspath::compare_paths(path, &old_path, &compare_options) == 0 {
                return Some(new_path.clone());
            }
            // Trim the directory prefix ourselves (rather than using
            // tspath.StartsWithDirectory followed by a separate slice on
            // len(oldPath)) so the containment check and the suffix we return can
            // never disagree, and so we don't slice path by a byte count derived
            // from a canonicalized/differently-cased string: case-folding can
            // change a path's UTF-8 byte length without changing its rune count
            // (e.g. the Kelvin sign 'K' folds to the single-byte 'k'), which
            // could otherwise put len(oldPath) out of range of path.
            if let Some(suffix) = tspath::trim_file_path_prefix(path, &trimmed_old_path, self.use_case_sensitive_file_names()) {
                if suffix.starts_with('/') || suffix.starts_with('\\') {
                    return Some(format!("{}{}", new_path, suffix));
                }
            }
            None
        })
    }

    // file_rename.go:109
    fn update_tsconfig_files(&self, program: &Program, change_tracker: &mut change::Tracker, old_to_new: &pathUpdater, old_path: &str, new_path: &str) {
        let command_line = program.command_line();
        let Some(config_file) = command_line.config_file else {
            return;
        };

        let config_file = config_file.source_file;
        let config_dir = tspath::get_directory_path(config_file.file_name());
        let Some(json_object_literal) = get_ts_config_object_literal_expression(Some(config_file)) else {
            return;
        };

        let use_case_sensitive_file_names = self.use_case_sensitive_file_names();
        let converters = Arc::clone(&self.converters);
        for_each_object_property(Some(json_object_literal), &mut |property, property_name| match property_name {
            "files" | "include" | "exclude" => {
                let found_exact_match =
                    update_paths_property(config_file, &config_dir, property, change_tracker, old_to_new, &converters, use_case_sensitive_file_names);
                let initializer = property.initializer().unwrap();
                if found_exact_match || property_name != "include" || !ast::is_array_literal_expression(initializer) {
                    return;
                }
                let (old_spec, is_default) = command_line.get_matched_include_spec(old_path);
                if !old_spec.is_empty() && !is_default {
                    let (new_spec, _) = command_line.get_matched_include_spec(new_path);
                    if new_spec.is_empty() {
                        let elements = initializer.elements();
                        if !elements.is_empty() {
                            let literal = change_tracker
                                .node_factory
                                .new_string_literal(alloc_str(&relative_path_from_directory(&config_dir, new_path, use_case_sensitive_file_names)), TokenFlags::None);
                            change_tracker.insert_node_after(config_file, elements[elements.len() - 1], literal);
                        }
                    }
                }
            }
            "compilerOptions" => {
                let initializer = property.initializer().unwrap();
                if !ast::is_object_literal_expression(initializer) {
                    return;
                }
                for_each_object_property(Some(initializer), &mut |property, property_name| {
                    if let Some(option) = tsoptions::COMMAND_LINE_COMPILER_OPTIONS_MAP.get(property_name) {
                        let element_option = option.elements();
                        if option.is_file_path || (option.kind == CommandLineOptionKind::List && element_option.is_some_and(|e| e.is_file_path)) {
                            update_paths_property(config_file, &config_dir, property, change_tracker, old_to_new, &converters, use_case_sensitive_file_names);
                            return;
                        }
                    }

                    let initializer = property.initializer().unwrap();
                    if property_name != "paths" || !ast::is_object_literal_expression(initializer) {
                        return;
                    }
                    for_each_object_property(Some(initializer), &mut |paths_property, _| {
                        let paths_initializer = paths_property.initializer().unwrap();
                        if !ast::is_array_literal_expression(paths_initializer) {
                            return;
                        }
                        for &element in paths_initializer.elements() {
                            try_update_config_string(config_file, &config_dir, element, change_tracker, old_to_new, &converters, use_case_sensitive_file_names);
                        }
                    });
                });
            }
            _ => {}
        });
    }

    // file_rename.go:212
    fn update_relative_path(&self, old_to_new: &pathUpdater, old_import_from_path: &str, new_import_from_path: &str, relative_specifier: &str) -> String {
        let old_absolute = tspath::normalize_path(&tspath::combine_paths(&tspath::get_directory_path(old_import_from_path), &[relative_specifier]));
        let new_absolute = old_to_new(&old_absolute).unwrap_or(old_absolute);
        relative_import_path_from_directory(&tspath::get_directory_path(new_import_from_path), &new_absolute, self.use_case_sensitive_file_names())
    }

    // file_rename.go:221
    fn update_imports_for_file_rename(&self, program: &Program, change_tracker: &mut change::Tracker, old_to_new: &pathUpdater) {
        let all_files = program.get_source_files();
        let mut checker = program.get_type_checker(&Context::background());
        let module_specifier_preferences = self.user_preferences().module_specifier_preferences();

        let mut moved_files: Vec<movedFile> = Vec::new();
        for &source_file in all_files {
            if let Some(new_file_name) = old_to_new(crate::lsconv::Script::original_file_name(&source_file)) {
                moved_files.push(movedFile { source_file, new_file_name });
            }
        }

        for &source_file in all_files {
            let old_file_name = crate::lsconv::Script::original_file_name(&source_file).to_string();
            let new_from_old = old_to_new(&old_file_name);
            let file_moved = new_from_old.is_some();
            let new_import_from_path = new_from_old.unwrap_or_else(|| old_file_name.clone());

            for r in source_file.referenced_files() {
                if !tspath::is_external_module_name_relative(&r.file_name) {
                    continue;
                }
                let updated = self.update_relative_path(old_to_new, &old_file_name, &new_import_from_path, &r.file_name);
                if updated != r.file_name {
                    change_tracker.replace_text_range_with_text(source_file, r.text_range, &updated);
                }
            }

            for &import_string_literal in source_file.imports() {
                let updated = self.get_updated_import_specifier(
                    program,
                    &mut checker,
                    source_file,
                    import_string_literal,
                    old_to_new,
                    &moved_files,
                    &new_import_from_path,
                    file_moved,
                    &module_specifier_preferences,
                );
                if !updated.is_empty() && updated != import_string_literal.text() {
                    change_tracker.replace_text_range_with_text(source_file, create_string_text_range(source_file, import_string_literal), &updated);
                }
            }
        }
    }

    // file_rename.go:264
    // We assume the source file did not move to a different program.
    fn get_updated_import_specifier(
        &self,
        program: &Program,
        checker: &mut Checker,
        source_file: P<SourceFile>, // old importing source file
        import_literal: P<Node>,
        old_to_new: &pathUpdater,
        moved_files: &[movedFile],
        new_import_from_path: &str,
        importing_source_file_moved: bool,
        user_preferences: &modulespecifiers::UserPreferences,
    ) -> String {
        let imported_module_symbol = checker.get_symbol_at_location_exported(import_literal);
        if is_ambient_module_symbol(imported_module_symbol) {
            return String::new();
        }

        let Some(target) = get_source_file_to_import(program, source_file, import_literal, old_to_new) else {
            // First fall back: try every file affected by the rename to see if any of them would match the import specifier, and if so, obtain the updated specifier for that file.
            let updated = get_updated_import_specifier_from_moved_source_files(program, source_file, import_literal, moved_files, new_import_from_path, user_preferences);
            if !updated.is_empty() && updated != import_literal.text() {
                return updated;
            }
            // Fall back to a regular path update for unresolved module.
            if tspath::is_external_module_name_relative(import_literal.text()) {
                return self.update_relative_path(old_to_new, source_file.file_name(), new_import_from_path, import_literal.text());
            }
            return String::new();
        };

        // Optimization: neither the importing or imported file changed.
        if !target.updated && !(importing_source_file_moved && tspath::is_external_module_name_relative(import_literal.text())) {
            return String::new();
        }

        modulespecifiers::update_module_specifier(
            &program.options(),
            program,
            source_file,
            new_import_from_path,
            import_literal.text(),
            &target.new_file_name,
            user_preferences.clone(),
            ModuleSpecifierOptions { override_import_mode: program.get_mode_for_usage_location(source_file, import_literal) },
        )
    }
}

// file_rename.go:162
fn update_paths_property(
    config_file: P<SourceFile>,
    config_dir: &str,
    property: P<Node>,
    change_tracker: &mut change::Tracker,
    old_to_new: &pathUpdater,
    converters: &Converters,
    use_case_sensitive_file_names: bool,
) -> bool {
    let initializer = property.initializer().unwrap();
    let elements: Vec<P<Node>> = if ast::is_array_literal_expression(initializer) { initializer.elements().to_vec() } else { vec![initializer] };

    let mut found_exact_match = false;
    for element in elements {
        found_exact_match = try_update_config_string(config_file, config_dir, element, change_tracker, old_to_new, converters, use_case_sensitive_file_names) || found_exact_match;
    }
    found_exact_match
}

// file_rename.go:175
fn try_update_config_string(
    config_file: P<SourceFile>,
    config_dir: &str,
    element: P<Node>,
    change_tracker: &mut change::Tracker,
    old_to_new: &pathUpdater,
    converters: &Converters,
    use_case_sensitive_file_names: bool,
) -> bool {
    if !ast::is_string_literal(element) {
        return false;
    }

    let element_file_name = tspath::normalize_path(&tspath::combine_paths(config_dir, &[element.text()]));
    let Some(updated) = old_to_new(&element_file_name) else {
        return false;
    };

    let text_range = TextRange::new(scanner::get_token_pos_of_node(element, config_file, false) + 1, element.end() - 1);
    let (lsp_range, fidelity) = converters.to_lsp_range(&config_file, text_range);
    assert!(fidelity.is_exact(), "config files are not content-mapped");
    change_tracker.replace_range_with_text(config_file, lsp_range, &relative_path_from_directory(config_dir, &updated, use_case_sensitive_file_names));
    true
}

// file_rename.go:310
fn get_source_file_to_import(program: &Program, source_file: P<SourceFile>, import_literal: P<Node>, old_to_new: &pathUpdater) -> Option<toImport> {
    if let Some(resolved) = program.get_resolved_module_from_module_specifier(source_file, import_literal) {
        if !resolved.resolved_file_name.is_empty() {
            let old_file_name = resolved.resolved_file_name.to_string();
            if let Some(new_file_name) = old_to_new(&old_file_name) {
                return Some(toImport { new_file_name, updated: true });
            }
            return Some(toImport { new_file_name: old_file_name, updated: false });
        }
    }

    None
}

// file_rename.go:329
// As a fall back for unresolved modules, we'll check every file affected by the rename to see if any of them would match
// the import specifier, and if so, we'll obtain the updated specifier for that file.
fn get_updated_import_specifier_from_moved_source_files(
    program: &Program,
    source_file: P<SourceFile>,
    import_literal: P<Node>,
    moved_files: &[movedFile],
    importing_source_file_name: &str,
    user_preferences: &modulespecifiers::UserPreferences,
) -> String {
    let resolution_mode = program.get_mode_for_usage_location(source_file, import_literal);
    for candidate in moved_files {
        let old_specifier = modulespecifiers::update_module_specifier(
            &program.options(),
            program,
            source_file,
            importing_source_file_name,
            import_literal.text(),
            candidate.source_file.file_name(),
            user_preferences.clone(),
            ModuleSpecifierOptions { override_import_mode: resolution_mode },
        );
        if old_specifier != import_literal.text() {
            continue;
        }

        return modulespecifiers::update_module_specifier(
            &program.options(),
            program,
            source_file,
            importing_source_file_name,
            import_literal.text(),
            &candidate.new_file_name,
            user_preferences.clone(),
            ModuleSpecifierOptions { override_import_mode: resolution_mode },
        );
    }
    String::new()
}

// file_rename.go:366
fn create_string_text_range(source_file: P<SourceFile>, node: P<Node>) -> TextRange {
    TextRange::new(scanner::get_token_pos_of_node(node, source_file, false) + 1, node.end() - 1)
}

// file_rename.go:370
fn get_ts_config_object_literal_expression(ts_config_source_file: Option<P<SourceFile>>) -> Option<P<Node>> {
    let ts_config_source_file = ts_config_source_file?;
    let statements = ts_config_source_file.statements.nodes();
    if !statements.is_empty() {
        let expression = statements[0].expression()?;
        if ast::is_object_literal_expression(expression) {
            return Some(expression);
        }
    }
    None
}

// file_rename.go:380
fn for_each_object_property(object_literal: Option<P<Node>>, cb: &mut dyn FnMut(P<Node>, &str)) {
    let Some(object_literal) = object_literal else {
        return;
    };
    for &property in object_literal.as_object_literal_expression().properties.nodes() {
        if !ast::is_property_assignment(property) {
            continue;
        }
        if let Some(name) = ast::try_get_text_of_property_name(property.name().unwrap()) {
            cb(property, &name);
        }
    }
}

// file_rename.go:394
fn relative_path_from_directory(from_directory: &str, to: &str, use_case_sensitive_file_names: bool) -> String {
    tspath::get_relative_path_from_directory(from_directory, to, &ComparePathsOptions { use_case_sensitive_file_names, ..Default::default() })
}

// file_rename.go:398
fn relative_import_path_from_directory(from_directory: &str, to: &str, use_case_sensitive_file_names: bool) -> String {
    tspath::ensure_path_is_non_module_name(&relative_path_from_directory(from_directory, to, use_case_sensitive_file_names))
}

// file_rename.go:402
fn is_ambient_module_symbol(symbol: Option<P<Symbol>>) -> bool {
    let Some(symbol) = symbol else {
        return false;
    };
    symbol.declarations().iter().any(|&d| ast::is_module_with_string_literal_name(d))
}
