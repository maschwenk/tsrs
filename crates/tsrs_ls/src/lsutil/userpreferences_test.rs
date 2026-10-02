use std::borrow::Cow;

use tsrs_core::collections::OrderedMap;
use tsrs_core::json::{self, Value};
use tsrs_core::Tristate;
use tsrs_modulespecifiers::{ImportModuleSpecifierEndingPreference, ImportModuleSpecifierPreference};

use super::*;

fn config(text: &str) -> OrderedMap<String, Value> {
    match json::unmarshal(text).unwrap() {
        Value::Object(m) => m,
        v => panic!("not an object: {v:?}"),
    }
}

fn marshal_prefs(p: &UserPreferences) -> String {
    json::marshal(&p.marshal_json_to().unwrap()).unwrap()
}

fn object<'a>(v: &'a OrderedMap<String, Value>, key: &str) -> &'a OrderedMap<String, Value> {
    match v.get(key) {
        Some(Value::Object(m)) => m,
        other => panic!("{key} is not an object: {other:?}"),
    }
}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

// userpreferences_test.go:13 (reflection over the struct fields -> the field table)
fn fill_non_zero_value(field: FieldMut<'_>) {
    match field {
        // core.Tristate is a uint8: SetUint(1)
        FieldMut::Tristate(f) => *f = Tristate::False,
        FieldMut::IndentStyle(f) => *f = IndentStyle(1),
        FieldMut::OrganizeImportsSort(f) => *f = OrganizeImportsSort::Ordinal,
        FieldMut::OrganizeImportsCaseFirst(f) => *f = OrganizeImportsCaseFirst::Lower,
        FieldMut::OrganizeImportsTypeOrder(f) => *f = OrganizeImportsTypeOrder::Last,
        FieldMut::Int(f) => *f = 1,
        FieldMut::OrganizeImportsCollation(f) => *f = OrganizeImportsCollation::Unicode,
        FieldMut::Bool(f) => *f = true,
        // getValidStringValue
        FieldMut::QuotePreference(f) => *f = QuotePreference::Single,
        FieldMut::JsxAttributeCompletionStyle(f) => *f = JsxAttributeCompletionStyle::Braces,
        FieldMut::IncludeInlayParameterNameHints(f) => *f = IncludeInlayParameterNameHints::All,
        FieldMut::SemicolonPreference(f) => *f = SemicolonPreference::Insert,
        FieldMut::ImportModuleSpecifierPreference(f) => *f = ImportModuleSpecifierPreference::Relative,
        FieldMut::ImportModuleSpecifierEndingPreference(f) => *f = ImportModuleSpecifierEndingPreference::Js,
        FieldMut::WorkspaceSymbolsScope(f) => *f = WorkspaceSymbolsScope(Cow::Borrowed("test")),
        FieldMut::String(f) => *f = "test".to_string(),
        FieldMut::Slice(f) => *f = Some(vec!["test".to_string()]),
    }
}

fn fill_non_zero_values(p: &mut UserPreferences) {
    for tag in USER_PREFERENCES_FIELD_TAGS {
        fill_non_zero_value((tag.get_mut)(p));
    }
}

fn is_zero(field: FieldRef<'_>) -> bool {
    match field {
        FieldRef::Tristate(f) => *f == Tristate::default(),
        FieldRef::IndentStyle(f) => *f == IndentStyle::default(),
        FieldRef::SemicolonPreference(f) => *f == SemicolonPreference::default(),
        FieldRef::QuotePreference(f) => *f == QuotePreference::default(),
        FieldRef::JsxAttributeCompletionStyle(f) => *f == JsxAttributeCompletionStyle::default(),
        FieldRef::IncludeInlayParameterNameHints(f) => *f == IncludeInlayParameterNameHints::default(),
        FieldRef::OrganizeImportsSort(f) => *f == OrganizeImportsSort::default(),
        FieldRef::OrganizeImportsCollation(f) => *f == OrganizeImportsCollation::default(),
        FieldRef::OrganizeImportsCaseFirst(f) => *f == OrganizeImportsCaseFirst::default(),
        FieldRef::OrganizeImportsTypeOrder(f) => *f == OrganizeImportsTypeOrder::default(),
        FieldRef::ImportModuleSpecifierPreference(f) => *f == ImportModuleSpecifierPreference::default(),
        FieldRef::ImportModuleSpecifierEndingPreference(f) => *f == ImportModuleSpecifierEndingPreference::default(),
        FieldRef::WorkspaceSymbolsScope(f) => *f == WorkspaceSymbolsScope::default(),
        FieldRef::Bool(f) => !*f,
        FieldRef::Int(f) => *f == 0,
        FieldRef::String(f) => f.is_empty(),
        FieldRef::Slice(f) => f.is_none(),
    }
}

// Go's collectFieldInfos panics on a field with neither tag and recurses into embedded structs, so every field of
// UserPreferences is reachable through a tag. The table must therefore address every field exactly once.
#[test]
fn test_user_preferences_field_table_covers_every_field() {
    let EditorSettings {
        base_indent_size: _,
        indent_size: _,
        tab_size: _,
        new_line_character: _,
        convert_tabs_to_spaces: _,
        indent_style: _,
        trim_trailing_whitespace: _,
    } = EditorSettings::default();
    let FormatCodeSettings {
        editor_settings: _,
        insert_space_after_comma_delimiter: _,
        insert_space_after_semicolon_in_for_statements: _,
        insert_space_before_and_after_binary_operators: _,
        insert_space_after_constructor: _,
        insert_space_after_keywords_in_control_flow_statements: _,
        insert_space_after_function_keyword_for_anonymous_functions: _,
        insert_space_after_opening_and_before_closing_nonempty_parenthesis: _,
        insert_space_after_opening_and_before_closing_nonempty_brackets: _,
        insert_space_after_opening_and_before_closing_nonempty_braces: _,
        insert_space_after_opening_and_before_closing_empty_braces: _,
        insert_space_after_opening_and_before_closing_template_string_braces: _,
        insert_space_after_opening_and_before_closing_jsx_expression_braces: _,
        insert_space_after_type_assertion: _,
        insert_space_before_function_parenthesis: _,
        place_open_brace_on_new_line_for_functions: _,
        place_open_brace_on_new_line_for_control_blocks: _,
        insert_space_before_type_annotation: _,
        indent_multi_line_object_literal_beginning_on_blank_line: _,
        semicolons: _,
        indent_switch_case: _,
    } = FormatCodeSettings::default();
    let InlayHintsPreferences {
        include_inlay_parameter_name_hints: _,
        include_inlay_parameter_name_hints_when_argument_matches_name: _,
        include_inlay_function_parameter_type_hints: _,
        include_inlay_variable_type_hints: _,
        include_inlay_variable_type_hints_when_type_matches_name: _,
        include_inlay_property_declaration_type_hints: _,
        include_inlay_function_like_return_type_hints: _,
        include_inlay_enum_member_value_hints: _,
    } = InlayHintsPreferences::default();
    let CodeLensUserPreferences {
        references_code_lens_enabled: _,
        implementations_code_lens_enabled: _,
        references_code_lens_show_on_all_functions: _,
        implementations_code_lens_show_on_interface_methods: _,
        implementations_code_lens_show_on_all_class_methods: _,
    } = CodeLensUserPreferences::default();
    let UserPreferences {
        format_code_settings: _,
        quote_preference: _,
        lazy_configured_projects_from_external_project: _,
        maximum_hover_length: _,
        include_completions_for_module_exports: _,
        include_completions_for_import_statements: _,
        include_automatic_optional_chain_completions: _,
        include_completions_with_class_member_snippets: _,
        include_completions_with_object_literal_method_snippets: _,
        jsx_attribute_completion_style: _,
        enable_auto_closing_tags: _,
        enable_jsdoc_completions: _,
        generate_return_in_doc_template: _,
        import_module_specifier_preference: _,
        import_module_specifier_ending: _,
        auto_import_specifier_exclude_regexes: _,
        auto_import_file_exclude_patterns: _,
        auto_import_entrypoint_directory_search: _,
        prefer_type_only_auto_imports: _,
        organize_imports_sort: _,
        organize_imports_ignore_case: _,
        organize_imports_collation: _,
        organize_imports_locale: _,
        organize_imports_numeric_collation: _,
        organize_imports_accent_collation: _,
        organize_imports_case_first: _,
        organize_imports_type_order: _,
        allow_text_changes_in_new_files: _,
        use_aliases_for_rename: _,
        allow_rename_of_import_path: _,
        provide_refactor_not_applicable_reason: _,
        inlay_hints: _,
        code_lens: _,
        prefer_go_to_source_definition: _,
        exclude_library_symbols_in_nav_to: _,
        workspace_symbols_scope: _,
        enable_formatting: _,
        enable_validation: _,
        disable_suggestions: _,
        disable_line_text_in_references: _,
        display_parts_for_jsdoc: _,
        report_style_checks_as_warnings: _,
        locale: _,
        disable_automatic_type_acquisition: _,
        automatic_type_acquisition_enabled: _,
        custom_config_file_name: _,
    } = UserPreferences::default();
    // Leaf fields: 7 (EditorSettings) + 20 (FormatCodeSettings) + 8 (InlayHints) + 5 (CodeLens) + 43 (UserPreferences
    // without its three struct fields).
    assert_eq!(USER_PREFERENCES_FIELD_TAGS.len(), 7 + 20 + 8 + 5 + 43);

    // Every entry has a tag (collectFieldInfos panics otherwise).
    assert_eq!(field_info_cache().len(), USER_PREFERENCES_FIELD_TAGS.len());

    // Every entry addresses its own field: filling one leaves all others at their zero value.
    for (i, tag) in USER_PREFERENCES_FIELD_TAGS.iter().enumerate() {
        let mut p = UserPreferences::default();
        fill_non_zero_value((tag.get_mut)(&mut p));
        for (j, other) in USER_PREFERENCES_FIELD_TAGS.iter().enumerate() {
            assert_eq!(is_zero((other.get)(&p)), i != j, "{} vs {}", tag.name, other.name);
        }
    }

    // Raw names are unique, so unstableNameIndex has one entry per raw name.
    let raw_count = field_info_cache().iter().filter(|info| !info.raw_name.is_empty()).count();
    assert_eq!(unstable_name_index().len(), raw_count);
}

// userpreferences_test.go:60
#[test]
fn test_user_preferences_roundtrip() {
    let mut original = UserPreferences::default();
    fill_non_zero_values(&mut original);

    let json_bytes = marshal_prefs(&original);

    // UnmarshalJSONFrom
    {
        let mut parsed = UserPreferences::default();
        parsed.unmarshal_json_from(&json::unmarshal(&json_bytes).unwrap()).unwrap();
        assert_eq!(original, parsed);
    }

    // withConfig
    {
        let config = config(&json_bytes);
        let parsed = UserPreferences::default().with_config(&config);
        assert_eq!(original, parsed);
    }
}

// userpreferences_test.go:87
#[test]
fn test_user_preferences_serialize() {
    // config path field serializes to nested path
    {
        let prefs = UserPreferences { quote_preference: QuotePreference::Single, ..Default::default() };
        let actual = config(&marshal_prefs(&prefs));
        let preferences = object(&actual, "preferences");
        assert_eq!(preferences.get("quoteStyle"), Some(&s("single")));
    }

    // raw-only field serializes to unstable section
    {
        let prefs = UserPreferences { disable_suggestions: Tristate::True, ..Default::default() };
        let actual = config(&marshal_prefs(&prefs));
        let unstable = object(&actual, "unstable");
        assert_eq!(unstable.get("disableSuggestions"), Some(&Value::Bool(true)));
    }

    // inlay hint inversion on serialize
    {
        let prefs = UserPreferences {
            inlay_hints: InlayHintsPreferences {
                include_inlay_parameter_name_hints: IncludeInlayParameterNameHints::All,
                include_inlay_parameter_name_hints_when_argument_matches_name: Tristate::True,
                ..Default::default()
            },
            ..Default::default()
        };
        let actual = config(&marshal_prefs(&prefs));
        let inlay_hints = object(&actual, "inlayHints");
        let parameter_names = object(inlay_hints, "parameterNames");
        assert_eq!(parameter_names.get("enabled"), Some(&s("all")));
        assert_eq!(parameter_names.get("suppressWhenArgumentMatchesName"), Some(&Value::Bool(false))); // inverted
    }

    // mixed config and unstable fields
    {
        let prefs = UserPreferences {
            quote_preference: QuotePreference::Single,
            disable_suggestions: Tristate::True,
            display_parts_for_jsdoc: Tristate::True,
            ..Default::default()
        };
        let actual = config(&marshal_prefs(&prefs));
        let preferences = object(&actual, "preferences");
        assert_eq!(preferences.get("quoteStyle"), Some(&s("single")));
        let unstable = object(&actual, "unstable");
        assert_eq!(unstable.get("disableSuggestions"), Some(&Value::Bool(true)));
        assert_eq!(unstable.get("displayPartsForJSDoc"), Some(&Value::Bool(true)));
    }
}

// json.Deterministic(true): object keys are written sorted at every level.
#[test]
fn test_user_preferences_serialize_deterministic() {
    let prefs = UserPreferences {
        quote_preference: QuotePreference::Single,
        disable_suggestions: Tristate::True,
        locale: "de".to_string(),
        format_code_settings: FormatCodeSettings {
            editor_settings: EditorSettings { tab_size: 2, base_indent_size: 4, ..Default::default() },
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        marshal_prefs(&prefs),
        concat!(
            r#"{"format":{"baseIndentSize":4,"tabSize":2},"locale":"de","#,
            r#""preferences":{"organizeImports":{"caseFirst":"default","sort":"auto","typeOrder":"auto","unicodeCollation":"ordinal"},"quoteStyle":"single"},"#,
            r#""unstable":{"disableSuggestions":true,"preferGoToSourceDefinition":false}}"#
        )
    );
}

// userpreferences_test.go:166
#[test]
fn test_user_preferences_parse_unstable() {
    let tests: Vec<(&str, &str, UserPreferences)> = vec![
        (
            "unstable fields with correct casing",
            r#"{
                "unstable": {
                    "disableSuggestions": true,
                    "maximumHoverLength": 100,
                    "allowRenameOfImportPath": true
                }
            }"#,
            UserPreferences {
                disable_suggestions: Tristate::True,
                maximum_hover_length: 100,
                allow_rename_of_import_path: Tristate::True,
                ..Default::default()
            },
        ),
        (
            "nested preferences path",
            r#"{
                "preferences": {
                    "quoteStyle": "single",
                    "useAliasesForRenames": true
                }
            }"#,
            UserPreferences { quote_preference: QuotePreference::Single, use_aliases_for_rename: Tristate::True, ..Default::default() },
        ),
        (
            "suggest section",
            r#"{
                "suggest": {
                    "autoImports": false,
                    "includeCompletionsForImportStatements": true
                }
            }"#,
            UserPreferences {
                include_completions_for_module_exports: Tristate::False,
                include_completions_for_import_statements: Tristate::True,
                ..Default::default()
            },
        ),
        (
            "inlayHints with invert",
            r#"{
                "inlayHints": {
                    "parameterNames": {
                        "enabled": "all",
                        "suppressWhenArgumentMatchesName": true
                    }
                }
            }"#,
            UserPreferences {
                inlay_hints: InlayHintsPreferences {
                    include_inlay_parameter_name_hints: IncludeInlayParameterNameHints::All,
                    include_inlay_parameter_name_hints_when_argument_matches_name: Tristate::False, // inverted
                    ..Default::default()
                },
                ..Default::default()
            },
        ),
        (
            "mixed config",
            r#"{
                "unstable": {
                    "displayPartsForJSDoc": true
                },
                "preferences": {
                    "importModuleSpecifier": "relative"
                },
                "workspaceSymbols": {
                    "excludeLibrarySymbols": true,
                    "scope": "currentProject"
                }
            }"#,
            UserPreferences {
                display_parts_for_jsdoc: Tristate::True,
                import_module_specifier_preference: ImportModuleSpecifierPreference::Relative,
                exclude_library_symbols_in_nav_to: Tristate::True,
                workspace_symbols_scope: WorkspaceSymbolsScope::CurrentProject,
                ..Default::default()
            },
        ),
        (
            "stable config overrides unstable",
            r#"{
                "unstable": {
                    "quotePreference": "double"
                },
                "preferences": {
                    "quoteStyle": "single"
                }
            }"#,
            UserPreferences {
                quote_preference: QuotePreference::Single, // stable wins
                ..Default::default()
            },
        ),
        (
            "unstable sets value when no stable config",
            r#"{
                "unstable": {
                    "includeAutomaticOptionalChainCompletions": false
                }
            }"#,
            UserPreferences { include_automatic_optional_chain_completions: Tristate::False, ..Default::default() },
        ),
        (
            "any field can be passed via unstable by its raw name",
            r#"{
                "unstable": {
                    "quotePreference": "double",
                    "includeCompletionsForModuleExports": true,
                    "excludeLibrarySymbolsInNavTo": true
                }
            }"#,
            UserPreferences {
                quote_preference: QuotePreference::Double,
                include_completions_for_module_exports: Tristate::True,
                exclude_library_symbols_in_nav_to: Tristate::True,
                ..Default::default()
            },
        ),
        (
            "TypeScript raw names work in unstable section",
            r#"{
                "unstable": {
                    "includeCompletionsForModuleExports": true,
                    "quotePreference": "single",
                    "providePrefixAndSuffixTextForRename": true,
                    "includeInlayParameterNameHints": "all",
                    "organizeImportsLocale": "en"
                }
            }"#,
            UserPreferences {
                include_completions_for_module_exports: Tristate::True,
                quote_preference: QuotePreference::Single,
                use_aliases_for_rename: Tristate::True,
                organize_imports_locale: "en".to_string(),
                inlay_hints: InlayHintsPreferences {
                    include_inlay_parameter_name_hints: IncludeInlayParameterNameHints::All,
                    ..Default::default()
                },
                ..Default::default()
            },
        ),
        (
            "old raw organize imports unicode preferences load as raw state",
            r#"{
                "unstable": {
                    "organizeImportsCollation": "unicode",
                    "organizeImportsCaseFirst": "upper",
                    "organizeImportsIgnoreCase": false,
                    "organizeImportsNumericCollation": true
                }
            }"#,
            UserPreferences {
                organize_imports_collation: OrganizeImportsCollation::Unicode,
                organize_imports_case_first: OrganizeImportsCaseFirst::Upper,
                organize_imports_ignore_case: Tristate::False,
                organize_imports_numeric_collation: Tristate::True,
                ..Default::default()
            },
        ),
        (
            "old top-level raw organize imports unicode preferences load as raw state",
            r#"{
                "organizeImportsCollation": "unicode",
                "organizeImportsIgnoreCase": true
            }"#,
            UserPreferences {
                organize_imports_collation: OrganizeImportsCollation::Unicode,
                organize_imports_ignore_case: Tristate::True,
                ..Default::default()
            },
        ),
        (
            "new top-level raw organize imports sort is accepted",
            r#"{
                "organizeImportsSort": "natural"
            }"#,
            UserPreferences { organize_imports_sort: OrganizeImportsSort::Natural, ..Default::default() },
        ),
        (
            "old raw organize imports ignore case loads as raw state",
            r#"{
                "unstable": {
                    "organizeImportsIgnoreCase": true
                }
            }"#,
            UserPreferences { organize_imports_ignore_case: Tristate::True, ..Default::default() },
        ),
        (
            "new raw organize imports sort loads alongside old raw preferences",
            r#"{
                "unstable": {
                    "organizeImportsSort": "ordinal",
                    "organizeImportsCollation": "unicode",
                    "organizeImportsIgnoreCase": true
                }
            }"#,
            UserPreferences {
                organize_imports_sort: OrganizeImportsSort::Ordinal,
                organize_imports_collation: OrganizeImportsCollation::Unicode,
                organize_imports_ignore_case: Tristate::True,
                ..Default::default()
            },
        ),
        (
            "old nested organize imports unicode preferences load as raw state",
            r#"{
                "preferences": {
                    "organizeImports": {
                        "unicodeCollation": "unicode",
                        "caseSensitivity": "caseSensitive",
                        "numericCollation": true,
                        "caseFirst": "upper"
                    }
                }
            }"#,
            UserPreferences {
                organize_imports_collation: OrganizeImportsCollation::Unicode,
                organize_imports_ignore_case: Tristate::False,
                organize_imports_numeric_collation: Tristate::True,
                organize_imports_case_first: OrganizeImportsCaseFirst::Upper,
                ..Default::default()
            },
        ),
        (
            "new nested organize imports sort loads alongside old nested preferences",
            r#"{
                "preferences": {
                    "organizeImports": {
                        "sort": "ordinalIgnoreCase",
                        "unicodeCollation": "unicode",
                        "caseSensitivity": "caseSensitive"
                    }
                }
            }"#,
            UserPreferences {
                organize_imports_sort: OrganizeImportsSort::OrdinalIgnoreCase,
                organize_imports_collation: OrganizeImportsCollation::Unicode,
                organize_imports_ignore_case: Tristate::False,
                ..Default::default()
            },
        ),
    ];

    for (name, text, expected) in tests {
        let config = config(text);
        let parsed = UserPreferences::default().with_config(&config);
        assert_eq!(expected, parsed, "{name}");
    }
}

// userpreferences_test.go:429
#[test]
fn test_user_preferences_locale() {
    let prefs = parse_user_preferences(&config(
        r#"{
            "typescript": {"locale": "de"},
            "js/ts": {"locale": "fr"}
        }"#,
    ));
    assert_eq!(prefs.locale, "fr");
}

// userpreferences_test.go:444
#[test]
fn test_user_preferences_report_style_checks_as_warnings() {
    // reportStyleChecksAsWarnings via config path
    {
        let prefs = parse_user_preferences(&config(r#"{"js/ts": {"reportStyleChecksAsWarnings": false}}"#));
        assert_eq!(prefs.report_style_checks_as_warnings, Tristate::False);
    }

    // reportStyleChecksAsWarnings defaults to true
    {
        let prefs = new_default_user_preferences();
        assert_eq!(prefs.report_style_checks_as_warnings, Tristate::True);
    }

    // reportStyleChecksAsWarnings via unstable section
    {
        let prefs = parse_user_preferences(&config(r#"{"js/ts": {"unstable": {"reportStyleChecksAsWarnings": false}}}"#));
        assert_eq!(prefs.report_style_checks_as_warnings, Tristate::False);
    }
}

// userpreferences_test.go:476
#[test]
fn test_user_preferences_parse_server_feature_preferences() {
    // preferred server feature settings
    {
        let prefs = parse_user_preferences(&config(
            r#"{
                "js/ts": {
                    "validate": {"enabled": false},
                    "format": {"enabled": false},
                    "autoClosingTags": {"enabled": false}
                }
            }"#,
        ));
        assert_eq!(prefs.enable_validation, Tristate::False);
        assert_eq!(prefs.enable_formatting, Tristate::False);
        assert_eq!(prefs.enable_auto_closing_tags, Tristate::False);
    }

    // legacy server feature fallbacks
    {
        let prefs = parse_user_preferences(&config(
            r#"{
                "typescript": {
                    "validate": {"enable": false},
                    "format": {"enable": false},
                    "autoClosingTags": false
                }
            }"#,
        ));
        assert_eq!(prefs.enable_validation, Tristate::False);
        assert_eq!(prefs.enable_formatting, Tristate::False);
        assert_eq!(prefs.enable_auto_closing_tags, Tristate::False);
    }

    // preferred settings take precedence over fallbacks
    {
        let prefs = parse_user_preferences(&config(
            r#"{
                "typescript": {
                    "validate": {"enable": false},
                    "format": {"enable": false},
                    "autoClosingTags": false
                },
                "js/ts": {
                    "validate": {"enabled": true},
                    "format": {"enabled": true},
                    "autoClosingTags": {"enabled": true}
                }
            }"#,
        ));
        assert_eq!(prefs.enable_validation, Tristate::True);
        assert_eq!(prefs.enable_formatting, Tristate::True);
        assert_eq!(prefs.enable_auto_closing_tags, Tristate::True);
    }
}

// userpreferences_test.go:531
#[test]
fn test_parse_user_preferences_editor_formatting() {
    let prefs = parse_user_preferences(&config(r#"{"editor": {"tabSize": 2, "insertSpaces": false}}"#));

    assert_eq!(prefs.format_code_settings.tab_size, 2);
    assert_eq!(prefs.format_code_settings.indent_size, 2);
    assert_eq!(prefs.format_code_settings.convert_tabs_to_spaces, Tristate::False);
}

// userpreferences_test.go:546
#[test]
fn test_user_preferences_parse_jsdoc_completion_preferences() {
    // unified jsdoc enabled setting
    {
        let prefs = parse_user_preferences(&config(r#"{"js/ts": {"suggest": {"jsdoc": {"enabled": false}}}}"#));
        assert_eq!(prefs.enable_jsdoc_completions, Tristate::False);
    }

    // language fallback completeJSDocs setting
    {
        let prefs = parse_user_preferences(&config(r#"{"typescript": {"suggest": {"completeJSDocs": false}}}"#));
        assert_eq!(prefs.enable_jsdoc_completions, Tristate::False);
    }

    // unified jsdoc enabled takes precedence over language fallback
    {
        let prefs = parse_user_preferences(&config(
            r#"{
                "typescript": {"suggest": {"completeJSDocs": false}},
                "js/ts": {"suggest": {"jsdoc": {"enabled": true}}}
            }"#,
        ));
        assert_eq!(prefs.enable_jsdoc_completions, Tristate::True);
    }

    // unified jsdoc generateReturns setting
    {
        let prefs = parse_user_preferences(&config(r#"{"js/ts": {"suggest": {"jsdoc": {"generateReturns": false}}}}"#));
        assert_eq!(prefs.generate_return_in_doc_template, Tristate::False);
    }

    // language jsdoc generateReturns setting
    {
        let prefs = parse_user_preferences(&config(r#"{"typescript": {"suggest": {"jsdoc": {"generateReturns": false}}}}"#));
        assert_eq!(prefs.generate_return_in_doc_template, Tristate::False);
    }
}

// userpreferences_test.go:623
#[test]
fn test_user_preferences_parse_ata() {
    // ParseUserPreferences with unified ATA setting in js/ts section
    {
        let prefs = parse_user_preferences(&config(r#"{"js/ts": {"tsserver": {"automaticTypeAcquisition": {"enabled": false}}}}"#));
        assert!(prefs.is_ata_disabled());
        assert_eq!(prefs.automatic_type_acquisition_enabled, Tristate::False);
    }

    // ParseUserPreferences with deprecated disableAutomaticTypeAcquisition in typescript section
    {
        let prefs = parse_user_preferences(&config(r#"{"typescript": {"disableAutomaticTypeAcquisition": true}}"#));
        assert!(prefs.is_ata_disabled());
        assert_eq!(prefs.disable_automatic_type_acquisition, Tristate::True);
    }

    // unified setting takes precedence over deprecated setting
    {
        // Both settings set: unified (js/ts) should take precedence
        let prefs = parse_user_preferences(&config(
            r#"{
                "typescript": {"disableAutomaticTypeAcquisition": true},
                "js/ts": {"tsserver": {"automaticTypeAcquisition": {"enabled": true}}}
            }"#,
        ));
        assert!(!prefs.is_ata_disabled());
        assert_eq!(prefs.automatic_type_acquisition_enabled, Tristate::True);
    }

    // IsATADisabled returns false when neither setting is configured
    {
        let prefs = new_default_user_preferences();
        assert!(!prefs.is_ata_disabled());
    }
}
