use std::rc::Rc;
use std::sync::Arc;

use rustc_hash::FxHashMap;
use tsrs_ast::{self as ast, Kind, Node, NodeFlags, SourceFile, TokenFlags};
use tsrs_compiler::Program;
use tsrs_core::context::Context;
use tsrs_core::tspath;
use tsrs_core::{alloc_str, CompilerOptions, ModuleDetectionKind, ModuleKind, TextRange, Tristate, P};
use tsrs_diagnostics as diagnostics;
use tsrs_lsproto as lsproto;
use tsrs_modulespecifiers::{self as modulespecifiers, ImportModuleSpecifierPreference, ResultKind};
use tsrs_scanner as scanner;

use super::export::{Export, ExportSyntax, ModuleID};
use super::util::try_get_module_id_and_file_name_of_module_symbol;
use super::view::View;
use crate::astnav;
use crate::change::{self, NodeOptions};
use crate::lsconv::Converters;
use crate::lsutil::{self, FormatCodeSettings, QuotePreference, UserPreferences};

// fix.go:30
#[derive(Clone, Debug, Default)]
pub(crate) struct newImportBinding {
    pub(crate) kind: lsproto::ImportKind,
    pub(crate) property_name: String,
    pub(crate) name: String,
    pub(crate) add_as_type_only: lsproto::AddAsTypeOnly,
}

// fix.go:37
#[derive(Clone, Debug, Default)]
pub struct Fix {
    pub auto_import_fix: lsproto::AutoImportFix,

    pub module_specifier_kind: ResultKind,
    pub is_re_export: bool,
    pub module_file_name: String,
    pub type_only_alias_declaration: Option<P<Node>>,
}

impl std::ops::Deref for Fix {
    type Target = lsproto::AutoImportFix;
    fn deref(&self) -> &lsproto::AutoImportFix {
        &self.auto_import_fix
    }
}

// fix.go:45
pub(crate) struct addToExistingImportFix {
    pub(crate) import_clause_or_binding_pattern: P<Node>,
    // One of `defaultImport` or `namedImports` will be present
    pub(crate) default_import: Option<newImportBinding>,
    pub(crate) named_import: Option<newImportBinding>,
}

impl Fix {
    // fix.go:55
    // Edits produces the text edits and a human-readable description for the fix. The returned bool is false
    // when the fix targets a content-mapped file and any edit could not be placed within a single verbatim
    // span, meaning it cannot be safely applied to the original text and the caller should discard it.
    pub fn edits(
        &self,
        ctx: &Context,
        file: P<SourceFile>,
        compiler_options: P<CompilerOptions>,
        format_options: FormatCodeSettings,
        converters: &Arc<Converters>,
        preferences: &UserPreferences,
    ) -> (Vec<lsproto::TextEdit>, String, bool) {
        let mut tracker = change::new_tracker(ctx, &compiler_options, format_options, converters.clone());
        match self.kind {
            lsproto::AutoImportFixKind::UseNamespace => {
                let description = add_namespace_qualifier(self, &mut tracker, file);
                let (edits, safe) = file_edits(&mut tracker, file);
                (edits, description, safe)
            }
            lsproto::AutoImportFixKind::AddToExisting => {
                if file.imports().len() <= self.import_index as usize {
                    panic!("import index out of range");
                }
                let existing_fix = get_add_to_existing_import_fix(file, self);
                add_to_existing_import(
                    &mut tracker,
                    file,
                    existing_fix.import_clause_or_binding_pattern,
                    existing_fix.default_import.as_ref(),
                    &existing_fix.named_import.into_iter().collect::<Vec<_>>(),
                    preferences,
                );
                let (edits, safe) = file_edits(&mut tracker, file);
                (edits, diagnostics::Update_import_from_0.localize(&[&self.module_specifier]), safe)
            }
            lsproto::AutoImportFixKind::AddNew => {
                let default_import = if self.import_kind == lsproto::ImportKind::Default {
                    Some(newImportBinding { name: self.name.clone(), add_as_type_only: self.add_as_type_only, ..Default::default() })
                } else {
                    None
                };
                let named_imports = if self.import_kind == lsproto::ImportKind::Named {
                    vec![newImportBinding { name: self.name.clone(), add_as_type_only: self.add_as_type_only, ..Default::default() }]
                } else {
                    Vec::new()
                };
                let mut namespace_like_import: Option<newImportBinding> = None;
                // qualification := f.qualification()
                if self.import_kind == lsproto::ImportKind::Namespace || self.import_kind == lsproto::ImportKind::CommonJS {
                    namespace_like_import = Some(newImportBinding { kind: self.import_kind, name: self.name.clone(), ..Default::default() });
                    // if qualification != nil && qualification.namespacePref != "" {
                    // 	namespaceLikeImport.name = qualification.namespacePref
                    // }
                }

                let quote_preference = lsutil::get_quote_preference(file, preferences);
                let declarations = if self.use_require {
                    get_new_requires(&tracker, &self.module_specifier, quote_preference, default_import.as_ref(), &named_imports, namespace_like_import.as_ref(), &compiler_options)
                } else {
                    get_new_imports(
                        &tracker,
                        &self.module_specifier,
                        quote_preference,
                        default_import.as_ref(),
                        &named_imports,
                        namespace_like_import.as_ref(),
                        &compiler_options,
                        preferences,
                    )
                };

                insert_imports(&mut tracker, file, &declarations, true /*blankLineBetween*/, preferences);
                // if qualification != nil {
                // 	addNamespaceQualifier(tracker, file, qualification)
                // }
                let (edits, safe) = file_edits(&mut tracker, file);
                (edits, diagnostics::Add_import_from_0.localize(&[&self.module_specifier]), safe)
            }
            lsproto::AutoImportFixKind::PromoteTypeOnly => {
                let promoted_declaration = promote_from_type_only(&mut tracker, self.type_only_alias_declaration.unwrap(), &compiler_options, file, preferences);
                if promoted_declaration.kind() == Kind::ImportSpecifier {
                    let module_spec = get_module_specifier_text(promoted_declaration.parent().unwrap().parent().unwrap());
                    let (edits, safe) = file_edits(&mut tracker, file);
                    return (edits, diagnostics::Remove_type_from_import_of_0_from_1.localize(&[&self.name, &module_spec]), safe);
                }
                let module_spec = get_module_specifier_text(promoted_declaration);
                let (edits, safe) = file_edits(&mut tracker, file);
                (edits, diagnostics::Remove_type_from_import_declaration_from_0.localize(&[&module_spec]), safe)
            }
            lsproto::AutoImportFixKind::JsdocTypeImport => {
                let description = add_import_type(self, file, preferences, &mut tracker);
                let (edits, safe) = file_edits(&mut tracker, file);
                (edits, description, safe)
            }
            _ => panic!("unimplemented fix edit"),
        }
    }
}

// fix.go:132
// fileEdits returns the edits recorded for file, along with whether they are safe to apply. GetChanges
// drops the edits of any content-mapped file that cannot be faithfully mapped back to the original text,
// so an empty result with safe == false means the fix could not be represented and must be discarded.
fn file_edits(tracker: &mut change::Tracker, file: P<SourceFile>) -> (Vec<lsproto::TextEdit>, bool) {
    let (mut changes, unmappable) = tracker.get_changes();
    (changes.shift_remove(crate::lsconv::Script::original_file_name(&file)).unwrap_or_default(), unmappable.is_empty())
}

// fix.go:137
pub(crate) fn add_import_type(f: &Fix, file: P<SourceFile>, preferences: &UserPreferences, tracker: &mut change::Tracker) -> String {
    let Some(usage_position) = f.usage_position else {
        panic!("UsagePosition must be set for JSDoc type import fix");
    };
    let quote_preference = lsutil::get_quote_preference(file, preferences);
    let mut quote_char = "\"";
    if quote_preference == QuotePreference::Single {
        quote_char = "'";
    }
    let import_type_prefix = format!("import({}{}{}).", quote_char, f.module_specifier, quote_char);
    tracker.insert_text(file, usage_position, &import_type_prefix);
    diagnostics::Change_0_to_1.localize(&[&f.name, &format!("{}{}", import_type_prefix, f.name)])
}

// fix.go:151
pub(crate) fn add_namespace_qualifier(f: &Fix, tracker: &mut change::Tracker, file: P<SourceFile>) -> String {
    let Some(usage_position) = f.usage_position.filter(|_| !f.namespace_prefix.is_empty()) else {
        panic!("namespace fix requires usage position and prefix");
    };
    let qualified = format!("{}.{}", f.namespace_prefix, f.name);
    tracker.insert_text(file, usage_position, &format!("{}.", f.namespace_prefix));
    diagnostics::Change_0_to_1.localize(&[&f.name, &qualified])
}

// fix.go:160
pub(crate) fn get_add_to_existing_import_fix(file: P<SourceFile>, fix: &Fix) -> addToExistingImportFix {
    if fix.kind != lsproto::AutoImportFixKind::AddToExisting {
        panic!("expected add to existing import fix");
    }
    let module_specifier = file.imports()[fix.import_index as usize];
    let Some(import_node) = ast::try_get_import_from_module_specifier(module_specifier) else {
        panic!("expected import declaration");
    };
    let import_clause_or_binding_pattern = match import_node.kind() {
        Kind::ImportDeclaration => match import_node.import_clause() {
            Some(c) => c,
            None => panic!("expected import clause"),
        },
        Kind::CallExpression => {
            if !ast::is_variable_declaration_initialized_to_require(import_node.parent().unwrap()) {
                panic!("expected require call expression to be in variable declaration");
            }
            match import_node.parent().unwrap().name() {
                Some(n) if ast::is_object_binding_pattern(n) => n,
                _ => panic!("expected object binding pattern in variable declaration"),
            }
        }
        _ => panic!("expected import declaration or require call expression"),
    };

    let default_import = if fix.import_kind == lsproto::ImportKind::Default {
        Some(newImportBinding { kind: lsproto::ImportKind::Default, name: fix.name.clone(), add_as_type_only: fix.add_as_type_only, ..Default::default() })
    } else {
        None
    };
    let named_imports = if fix.import_kind == lsproto::ImportKind::Named {
        Some(newImportBinding { kind: lsproto::ImportKind::Named, name: fix.name.clone(), add_as_type_only: fix.add_as_type_only, ..Default::default() })
    } else {
        None
    };
    addToExistingImportFix { import_clause_or_binding_pattern, default_import, named_import: named_imports }
}

// fix.go:200
pub(crate) fn add_to_existing_import(
    ct: &mut change::Tracker,
    file: P<SourceFile>,
    import_clause_or_binding_pattern: P<Node>,
    default_import: Option<&newImportBinding>,
    named_imports: &[newImportBinding],
    preferences: &UserPreferences,
) {
    match import_clause_or_binding_pattern.kind() {
        Kind::ObjectBindingPattern => {
            if let Some(default_import) = default_import {
                add_element_to_binding_pattern(ct, file, import_clause_or_binding_pattern, &default_import.name, "default");
            }
            for named_import in named_imports {
                add_element_to_binding_pattern(ct, file, import_clause_or_binding_pattern, &named_import.name, "");
            }
        }
        Kind::ImportClause => {
            let import_clause = import_clause_or_binding_pattern.as_import_clause();
            let clause_is_type_only = import_clause_or_binding_pattern.is_type_only();

            // promoteFromTypeOnly = true if we need to promote the entire original clause from type only
            let promote_from_type_only = clause_is_type_only
                && (named_imports.iter().any(|i| i.add_as_type_only == lsproto::AddAsTypeOnly::NotAllowed)
                    || default_import.is_some_and(|i| i.add_as_type_only == lsproto::AddAsTypeOnly::NotAllowed));

            let mut existing_specifiers: Vec<P<Node>> = Vec::new();
            if let Some(nb) = import_clause.named_bindings {
                if nb.kind() == Kind::NamedImports {
                    existing_specifiers = nb.elements().to_vec();
                }
            }

            if let Some(default_import) = default_import {
                assert!(import_clause.name().is_none(), "Cannot add a default import to an import clause that already has one");
                let id = ct.node_factory.new_identifier(alloc_str(&default_import.name));
                ct.insert_node_at(
                    file,
                    astnav::get_start_of_node(import_clause_or_binding_pattern, file, false),
                    id,
                    NodeOptions { suffix: ", ".to_string(), ..Default::default() },
                );
            }

            if !named_imports.is_empty() {
                let (specifier_comparer, is_sorted) =
                    lsutil::get_named_import_specifier_comparer_with_detection(import_clause_or_binding_pattern.parent().unwrap(), Some(file), preferences);
                let mut new_specifiers: Vec<P<Node>> = named_imports
                    .iter()
                    .map(|named_import| {
                        let mut identifier: Option<P<Node>> = None;
                        if !named_import.property_name.is_empty() {
                            identifier = Some(ct.node_factory.new_identifier(alloc_str(&named_import.property_name)));
                        }
                        ct.node_factory.new_import_specifier(
                            (!clause_is_type_only || promote_from_type_only) && should_use_type_only(named_import.add_as_type_only, preferences),
                            identifier,
                            ct.node_factory.new_identifier(alloc_str(&named_import.name)),
                        )
                    })
                    .collect();
                tsrs_core::goslices::sort_func(&mut new_specifiers, |&a, &b| specifier_comparer(a, b));
                if !existing_specifiers.is_empty() && is_sorted != Tristate::False {
                    // The sorting preference computed earlier may or may not have validated that these particular
                    // import specifiers are sorted. If they aren't, `getImportSpecifierInsertionIndex` will return
                    // nonsense. So if there are existing specifiers, even if we know the sorting preference, we
                    // need to ensure that the existing specifiers are sorted according to the preference in order
                    // to do a sorted insertion.

                    // If we're promoting the clause from type-only, we need to transform the existing imports
                    // before attempting to insert the new named imports (for comparison purposes only)
                    let mut specs_to_compare_against = existing_specifiers.clone();
                    if promote_from_type_only && !existing_specifiers.is_empty() {
                        specs_to_compare_against = existing_specifiers
                            .iter()
                            .map(|&e| {
                                let spec = e.as_import_specifier();
                                let property_name = spec.property_name();
                                ct.node_factory.new_import_specifier(
                                    true, // isTypeOnly
                                    property_name,
                                    spec.name(),
                                )
                            })
                            .collect();
                    }

                    for spec in new_specifiers {
                        let insertion_index = lsutil::get_import_specifier_insertion_index(&specs_to_compare_against, spec, &*specifier_comparer);
                        ct.insert_import_specifier_at_index(file, spec, import_clause.named_bindings.unwrap(), insertion_index);
                    }
                } else if !existing_specifiers.is_empty() {
                    for spec in new_specifiers {
                        ct.insert_node_in_list_after(file, existing_specifiers[existing_specifiers.len() - 1], spec, None);
                    }
                } else if !new_specifiers.is_empty() {
                    let named_imports = ct.node_factory.new_named_imports(ct.node_factory.new_node_list(new_specifiers));
                    if let Some(nb) = import_clause.named_bindings {
                        ct.replace_node(file, nb, named_imports, None);
                    } else {
                        let Some(name) = import_clause.name() else {
                            panic!("Import clause must have either named imports or a default import");
                        };
                        ct.insert_node_after(file, name, named_imports);
                    }
                }
            }

            if promote_from_type_only {
                // Delete the 'type' keyword from the import clause
                let type_keyword = get_type_keyword_of_type_only_import(import_clause_or_binding_pattern, file);
                ct.delete(file, type_keyword);

                // Add 'type' modifier to existing specifiers (not newly added ones)
                // We preserve the type-onlyness of existing specifiers regardless of whether
                // it would make a difference in emit (user preference).
                for &specifier in &existing_specifiers {
                    if !specifier.as_import_specifier().is_type_only {
                        ct.insert_modifier_before(file, Kind::TypeKeyword, specifier);
                    }
                }
            }
        }
        _ => panic!("Unsupported clause kind: {:?} for addToExistingImport", import_clause_or_binding_pattern.kind()),
    }
}

// fix.go:328
fn get_type_keyword_of_type_only_import(import_clause: P<Node>, source_file: P<SourceFile>) -> P<Node> {
    assert!(import_clause.is_type_only(), "import clause must be type-only");
    // The first child of a type-only import clause is the 'type' keyword
    // import type { foo } from './bar'
    //        ^^^^
    let type_keyword = astnav::find_child_of_kind(import_clause, Kind::TypeKeyword, source_file);
    assert!(type_keyword.is_some(), "type-only import clause should have a type keyword");
    type_keyword.unwrap()
}

// fix.go:338
fn add_element_to_binding_pattern(ct: &mut change::Tracker, file: P<SourceFile>, binding_pattern: P<Node>, name: &str, property_name: &str) {
    let property = if property_name.is_empty() { None } else { Some(ct.node_factory.new_identifier(alloc_str(property_name))) };
    let element = ct.node_factory.new_binding_element(None, None, Some(ct.node_factory.new_identifier(alloc_str(name))), property);
    let elements = binding_pattern.as_binding_pattern().elements;
    if !elements.nodes().is_empty() {
        ct.insert_node_in_list_after(file, elements.nodes()[elements.nodes().len() - 1], element, Some(elements));
    } else {
        let pattern = ct.node_factory.new_binding_pattern(Kind::ObjectBindingPattern, ct.node_factory.new_node_list(vec![element]));
        ct.replace_node(file, binding_pattern, pattern, None);
    }
}

// fix.go:353
pub(crate) fn get_new_imports(
    ct: &change::Tracker,
    module_specifier: &str,
    quote_preference: QuotePreference,
    default_import: Option<&newImportBinding>,
    named_imports: &[newImportBinding],
    namespace_like_import: Option<&newImportBinding>, // { lsproto.importKind: lsproto.ImportKind.CommonJS | lsproto.ImportKind.Namespace; }
    compiler_options: &CompilerOptions,
    preferences: &UserPreferences,
) -> Vec<P<Node>> {
    let token_flags = if quote_preference == QuotePreference::Single { TokenFlags::SingleQuote } else { TokenFlags::None };
    let module_specifier_string_literal = ct.node_factory.new_string_literal(alloc_str(module_specifier), token_flags);
    let mut statements: Vec<P<Node>> = Vec::new();
    if default_import.is_some() || !named_imports.is_empty() {
        // `verbatimModuleSyntax` should prefer top-level `import type` -
        // even though it's not an error, it would add unnecessary runtime emit.
        let top_level_type_only = default_import.is_none_or(|d| needs_type_only(d.add_as_type_only))
            && named_imports.iter().all(|i| needs_type_only(i.add_as_type_only))
            || (compiler_options.verbatim_module_syntax.is_true() || preferences.prefer_type_only_auto_imports.is_true())
                && default_import.is_none_or(|d| d.add_as_type_only != lsproto::AddAsTypeOnly::NotAllowed)
                && !named_imports.iter().any(|i| i.add_as_type_only == lsproto::AddAsTypeOnly::NotAllowed);

        let default_import_node = default_import.map(|d| ct.node_factory.new_identifier(alloc_str(&d.name)));

        let specifiers: Vec<P<Node>> = named_imports
            .iter()
            .map(|named_import| {
                let named_import_property_name =
                    if named_import.property_name.is_empty() { None } else { Some(ct.node_factory.new_identifier(alloc_str(&named_import.property_name))) };
                ct.node_factory.new_import_specifier(
                    !top_level_type_only && should_use_type_only(named_import.add_as_type_only, preferences),
                    named_import_property_name,
                    ct.node_factory.new_identifier(alloc_str(&named_import.name)),
                )
            })
            .collect();
        statements.push(make_import(ct, default_import_node, specifiers, module_specifier_string_literal, top_level_type_only));
    }

    if let Some(namespace_like_import) = namespace_like_import {
        let declaration = if namespace_like_import.kind == lsproto::ImportKind::CommonJS {
            ct.node_factory.new_import_equals_declaration(
                None, /*modifiers*/
                should_use_type_only(namespace_like_import.add_as_type_only, preferences),
                ct.node_factory.new_identifier(alloc_str(&namespace_like_import.name)),
                ct.node_factory.new_external_module_reference(module_specifier_string_literal),
            )
        } else {
            ct.node_factory.new_import_declaration(
                None, /*modifiers*/
                Some(ct.node_factory.new_import_clause(
                    /*phaseModifier*/ if should_use_type_only(namespace_like_import.add_as_type_only, preferences) { Kind::TypeKeyword } else { Kind::Unknown },
                    None, /*name*/
                    Some(ct.node_factory.new_namespace_import(ct.node_factory.new_identifier(alloc_str(&namespace_like_import.name)))),
                )),
                module_specifier_string_literal,
                None, /*attributes*/
            )
        };
        statements.push(declaration);
    }
    if statements.is_empty() {
        panic!("No statements to insert for new imports");
    }
    statements
}

// fix.go:414
pub(crate) fn get_new_requires(
    change_tracker: &change::Tracker,
    module_specifier: &str,
    quote_preference: QuotePreference,
    default_import: Option<&newImportBinding>,
    named_imports: &[newImportBinding],
    namespace_like_import: Option<&newImportBinding>,
    _compiler_options: &CompilerOptions,
) -> Vec<P<Node>> {
    let f = &change_tracker.node_factory;
    let quoted_module_specifier = f.new_string_literal(
        alloc_str(module_specifier),
        if quote_preference == QuotePreference::Single { TokenFlags::SingleQuote } else { TokenFlags::None },
    );
    let mut statements: Vec<P<Node>> = Vec::new();

    // const { default: foo, bar, etc } = require('./mod');
    if default_import.is_some() || !named_imports.is_empty() {
        let mut binding_elements: Vec<P<Node>> = Vec::new();
        for named_import in named_imports {
            let property_name = if named_import.property_name.is_empty() { None } else { Some(f.new_identifier(alloc_str(&named_import.property_name))) };
            binding_elements.push(f.new_binding_element(
                None, /*dotDotDotToken*/
                property_name,
                Some(f.new_identifier(alloc_str(&named_import.name))),
                None, /*initializer*/
            ));
        }
        if let Some(default_import) = default_import {
            binding_elements.insert(
                0,
                f.new_binding_element(
                    None, /*dotDotDotToken*/
                    Some(f.new_identifier("default")),
                    Some(f.new_identifier(alloc_str(&default_import.name))),
                    None, /*initializer*/
                ),
            );
        }
        let declaration = create_const_equals_require_declaration(
            change_tracker,
            f.new_binding_pattern(Kind::ObjectBindingPattern, f.new_node_list(binding_elements)),
            quoted_module_specifier,
        );
        statements.push(declaration);
    }

    // const foo = require('./mod');
    if let Some(namespace_like_import) = namespace_like_import {
        let declaration = create_const_equals_require_declaration(change_tracker, f.new_identifier(alloc_str(&namespace_like_import.name)), quoted_module_specifier);
        statements.push(declaration);
    }

    assert!(!statements.is_empty());
    statements
}

// fix.go:480
fn create_const_equals_require_declaration(change_tracker: &change::Tracker, name: P<Node>, quoted_module_specifier: P<Node>) -> P<Node> {
    let f = &change_tracker.node_factory;
    f.new_variable_statement(
        None, /*modifiers*/
        f.new_variable_declaration_list(
            f.new_node_list(vec![f.new_variable_declaration(
                name,
                None, /*exclamationToken*/
                None, /*type*/
                Some(f.new_call_expression(
                    f.new_identifier("require"),
                    None, /*questionDotToken*/
                    None, /*typeArguments*/
                    f.new_node_list(vec![quoted_module_specifier]),
                    NodeFlags::None,
                )),
            )]),
            NodeFlags::Const,
        ),
    )
}

// fix.go:503
pub(crate) fn insert_imports(ct: &mut change::Tracker, source_file: P<SourceFile>, imports: &[P<Node>], blank_line_between: bool, preferences: &UserPreferences) {
    let existing_import_statements: Vec<P<Node>> = if imports[0].kind() == Kind::VariableStatement {
        source_file.statements.nodes().iter().copied().filter(|&s| ast::is_require_variable_statement(s)).collect()
    } else {
        source_file.statements.nodes().iter().copied().filter(|&s| ast::is_any_import_syntax(s)).collect()
    };
    let (comparer, is_sorted) = lsutil::get_organize_imports_string_comparer_with_detection(&existing_import_statements, preferences);
    let comparer = comparer.expect("nil comparer");
    let mut sorted_new_imports = imports.to_vec();
    tsrs_core::goslices::sort_func(&mut sorted_new_imports, |&a, &b| lsutil::compare_imports_or_require_statements(a, b, &*comparer));

    if !existing_import_statements.is_empty() && is_sorted {
        // Existing imports are sorted, insert each new import at the correct position
        for &new_import in &sorted_new_imports {
            let insertion_index = lsutil::get_import_declaration_insert_index(&existing_import_statements, new_import, &|a, b| {
                lsutil::compare_imports_or_require_statements(a, b, &*comparer)
            });
            if insertion_index == 0 {
                // If the first import is top-of-file, insert after the leading comment which is likely the header.
                let mut leading_trivia_option = change::LeadingTriviaOption::None;
                if existing_import_statements[0] == source_file.statements.nodes()[0] {
                    leading_trivia_option = change::LeadingTriviaOption::Exclude;
                }
                ct.insert_node_before(source_file, existing_import_statements[0], new_import, false /*blankLineBetween*/, leading_trivia_option);
            } else {
                let prev_import = existing_import_statements[insertion_index - 1];
                ct.insert_node_after(source_file, prev_import, new_import);
            }
        }
    } else if !existing_import_statements.is_empty() {
        ct.insert_nodes_after(source_file, existing_import_statements[existing_import_statements.len() - 1], sorted_new_imports);
    } else {
        ct.insert_at_top_of_file(source_file, &sorted_new_imports, blank_line_between);
    }
}

// fix.go:541
fn make_import(ct: &change::Tracker, default_import: Option<P<Node>>, named_imports: Vec<P<Node>>, module_specifier: P<Node>, is_type_only: bool) -> P<Node> {
    let f = &ct.node_factory;
    let mut new_named_imports: Option<P<Node>> = None;
    if !named_imports.is_empty() {
        new_named_imports = Some(f.new_named_imports(f.new_node_list(named_imports)));
    }
    let mut import_clause: Option<P<Node>> = None;
    if default_import.is_some() || new_named_imports.is_some() {
        import_clause = Some(f.new_import_clause(if is_type_only { Kind::TypeKeyword } else { Kind::Unknown }, default_import, new_named_imports));
    }
    f.new_import_declaration(None /*modifiers*/, import_clause, module_specifier, None /*attributes*/)
}

impl View {
    // fix.go:554
    pub fn get_fixes(&self, export: &Export, for_jsx: bool, is_valid_type_only_use_site: bool, usage_position: Option<lsproto::Position>) -> Vec<Arc<Fix>> {
        let mut fixes: Vec<Arc<Fix>> = Vec::new();
        if let Some(namespace_fix) = self.try_use_existing_namespace_import(export, usage_position) {
            fixes.push(Arc::new(namespace_fix));
        }

        if let Some(fix) = self.try_add_to_existing_import(export, is_valid_type_only_use_site) {
            fixes.push(Arc::new(fix));
            return fixes;
        }

        // !!! getNewImportFromExistingSpecifier - even worth it?

        let (module_specifier, module_specifier_kind) = self.get_module_specifier(export, &self.preferences);
        if module_specifier.is_empty() {
            return fixes;
        }

        // Check if we need a JSDoc import type fix (for JS files with type-only imports)
        let is_js = tspath::has_js_file_extension(self.importing_file.file_name());
        let imported_symbol_has_value_meaning = export.flags.intersects(ast::SymbolFlags::Value) || export.is_unresolved_alias();
        if !imported_symbol_has_value_meaning && is_js && usage_position.is_some() {
            // For pure types in JS files, use JSDoc import type syntax
            return vec![Arc::new(Fix {
                auto_import_fix: lsproto::AutoImportFix {
                    kind: lsproto::AutoImportFixKind::JsdocTypeImport,
                    module_specifier,
                    name: export.name().to_string(),
                    usage_position,
                    ..Default::default()
                },
                module_specifier_kind,
                is_re_export: export.target.module_id != export.module_id,
                module_file_name: export.module_file_name.clone(),
                type_only_alias_declaration: None,
            })];
        }

        let import_kind = get_import_kind(self.importing_file, export, self.program, false /*forceImportKeyword*/);
        let add_as_type_only = get_add_as_type_only(is_valid_type_only_use_site, export, &self.program.options());

        let mut name = export.name().to_string();
        let starts_with_upper = tsrs_core::stringutil::unicode_is_upper(name.as_bytes()[0] as i32);
        if for_jsx && !starts_with_upper {
            if export.is_renameable() {
                let first = name.as_bytes()[0] as i32;
                let upper = char::from_u32(tsrs_core::stringutil::unicode_to_upper(first) as u32).unwrap();
                name = format!("{}{}", upper, &name[1..]);
            } else {
                return Vec::new();
            }
        }

        fixes.push(Arc::new(Fix {
            auto_import_fix: lsproto::AutoImportFix {
                kind: lsproto::AutoImportFixKind::AddNew,
                import_kind,
                module_specifier,
                name,
                use_require: self.should_use_require(),
                add_as_type_only,
                ..Default::default()
            },
            module_specifier_kind,
            is_re_export: export.target.module_id != export.module_id,
            module_file_name: export.module_file_name.clone(),
            type_only_alias_declaration: None,
        }));
        fixes
    }
}

// fix.go:621
// getAddAsTypeOnly determines if an import should be type-only based on usage context
fn get_add_as_type_only(is_valid_type_only_use_site: bool, export: &Export, compiler_options: &CompilerOptions) -> lsproto::AddAsTypeOnly {
    if !is_valid_type_only_use_site {
        // Can't use a type-only import if the usage is an emitting position
        return lsproto::AddAsTypeOnly::NotAllowed;
    }
    if compiler_options.verbatim_module_syntax.is_true() && (export.is_type_only || !export.flags.intersects(ast::SymbolFlags::Value))
        || export.is_type_only && export.flags.intersects(ast::SymbolFlags::Value)
    {
        // A type-only import is required for this symbol if under verbatimModuleSyntax and it's purely a type
        return lsproto::AddAsTypeOnly::Required;
    }
    lsproto::AddAsTypeOnly::Allowed
}

impl View {
    // fix.go:635
    fn try_use_existing_namespace_import(&self, export: &Export, usage_position: Option<lsproto::Position>) -> Option<Fix> {
        let usage_position = usage_position?;

        if get_import_kind(self.importing_file, export, self.program, false /*forceImportKeyword*/) != lsproto::ImportKind::Named {
            return None;
        }

        let existing_imports = self.get_existing_imports();
        let matching_declarations = existing_imports.get(&export.module_id).map(|v| v.as_slice()).unwrap_or(&[]);
        for existing_import in matching_declarations {
            let namespace_prefix = get_namespace_like_import_text(existing_import.node);
            if namespace_prefix.is_empty() || existing_import.module_specifier.is_empty() {
                continue;
            }
            return Some(Fix {
                auto_import_fix: lsproto::AutoImportFix {
                    kind: lsproto::AutoImportFixKind::UseNamespace,
                    name: export.name().to_string(),
                    module_specifier: existing_import.module_specifier.clone(),
                    import_kind: lsproto::ImportKind::Namespace,
                    add_as_type_only: lsproto::AddAsTypeOnly::Allowed,
                    import_index: existing_import.index as i32,
                    usage_position: Some(usage_position),
                    namespace_prefix,
                    ..Default::default()
                },
                ..Default::default()
            });
        }

        None
    }
}

// fix.go:668
fn get_namespace_like_import_text(declaration: P<Node>) -> String {
    match declaration.kind() {
        Kind::VariableDeclaration => {
            if let Some(name) = declaration.name() {
                if name.kind() == Kind::Identifier {
                    return name.text().to_string();
                }
            }
            String::new()
        }
        Kind::ImportEqualsDeclaration => declaration.name().unwrap().text().to_string(),
        Kind::JSDocImportTag | Kind::ImportDeclaration => {
            if let Some(import_clause) = declaration.import_clause() {
                if let Some(nb) = import_clause.as_import_clause().named_bindings {
                    if nb.kind() == Kind::NamespaceImport {
                        return nb.name().unwrap().text().to_string();
                    }
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

impl View {
    // fix.go:690
    fn try_add_to_existing_import(&self, export: &Export, is_valid_type_only_use_site: bool) -> Option<Fix> {
        let existing_imports = self.get_existing_imports();
        let matching_declarations = existing_imports.get(&export.module_id).map(|v| v.as_slice()).unwrap_or(&[]);
        if matching_declarations.is_empty() {
            return None;
        }

        // Can't use an es6 import for a type in JS.
        if ast::is_source_file_js(self.importing_file)
            && !export.flags.intersects(ast::SymbolFlags::Value)
            && !matching_declarations.iter().all(|i| ast::is_jsdoc_import_tag(i.node))
        {
            return None;
        }

        let import_kind = get_import_kind(self.importing_file, export, self.program, false /*forceImportKeyword*/);
        if import_kind == lsproto::ImportKind::CommonJS || import_kind == lsproto::ImportKind::Namespace {
            return None;
        }

        let add_as_type_only = get_add_as_type_only(is_valid_type_only_use_site, export, &self.program.options());

        let mut best: Option<Fix> = None;
        for existing_import in matching_declarations {
            if existing_import.node.kind() == Kind::ImportEqualsDeclaration {
                continue;
            }

            if existing_import.node.kind() == Kind::VariableDeclaration {
                if (import_kind == lsproto::ImportKind::Named || import_kind == lsproto::ImportKind::Default)
                    && existing_import.node.name().unwrap().kind() == Kind::ObjectBindingPattern
                {
                    let fix = Fix {
                        auto_import_fix: lsproto::AutoImportFix {
                            kind: lsproto::AutoImportFixKind::AddToExisting,
                            name: export.name().to_string(),
                            import_kind,
                            import_index: existing_import.index as i32,
                            module_specifier: existing_import.module_specifier.clone(),
                            add_as_type_only,
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    // Variable declarations are never type-only.
                    // Give preference to putting types in existing type-only imports and avoiding conversions
                    // of import statements to/from type-only.
                    if add_as_type_only == lsproto::AddAsTypeOnly::NotAllowed {
                        return Some(fix);
                    }
                    if best.is_none() {
                        best = Some(fix);
                    }
                }
                continue;
            }

            let Some(import_clause_node) = existing_import.node.import_clause() else {
                // Side-effect import (no import clause) - can't add to it
                continue;
            };
            if !ast::is_string_literal_like(existing_import.node.module_specifier().unwrap()) {
                // Side-effect import (no import clause) - can't add to it
                continue;
            }
            let import_clause = import_clause_node.as_import_clause();

            let named_bindings = import_clause.named_bindings;
            // A type-only import may not have both a default and named imports, so the only way a name can
            // be added to an existing type-only import is adding a named import to existing named bindings.
            if import_clause_node.is_type_only() && !(import_kind == lsproto::ImportKind::Named && named_bindings.is_some()) {
                continue;
            }

            if import_kind == lsproto::ImportKind::Default
                && (import_clause.name().is_some() ||
                // Cannot add a default import as type-only if the import already has named bindings
                add_as_type_only == lsproto::AddAsTypeOnly::Required && named_bindings.is_some())
            {
                continue;
            }

            // Cannot add a named import to a declaration that has a namespace import
            if import_kind == lsproto::ImportKind::Named && named_bindings.is_some_and(|nb| nb.kind() == Kind::NamespaceImport) {
                continue;
            }

            let fix = Fix {
                auto_import_fix: lsproto::AutoImportFix {
                    kind: lsproto::AutoImportFixKind::AddToExisting,
                    name: export.name().to_string(),
                    import_kind,
                    import_index: existing_import.index as i32,
                    module_specifier: existing_import.module_specifier.clone(),
                    add_as_type_only,
                    ..Default::default()
                },
                ..Default::default()
            };

            let is_type_only = import_clause_node.is_type_only();
            // Give preference to putting types in existing type-only imports and avoiding conversions
            // of import statements to/from type-only.
            if (add_as_type_only != lsproto::AddAsTypeOnly::NotAllowed && is_type_only) || (add_as_type_only == lsproto::AddAsTypeOnly::NotAllowed && !is_type_only)
            {
                return Some(fix);
            }
            if best.is_none() {
                best = Some(fix);
            }
        }

        best
    }
}

// fix.go:796
pub fn get_import_kind_for_import_statement(importing_file: P<SourceFile>, export: &Export, program: &Program) -> lsproto::ImportKind {
    get_import_kind(importing_file, export, program, true /*forceImportKeyword*/)
}

// fix.go:800
fn get_import_kind(importing_file: P<SourceFile>, export: &Export, program: &Program, force_import_keyword: bool) -> lsproto::ImportKind {
    if program.options().verbatim_module_syntax.is_true() && program.get_emit_module_format_of_file(importing_file) == ModuleKind::CommonJS {
        return lsproto::ImportKind::CommonJS;
    }
    match export.syntax {
        ExportSyntax::DefaultModifier | ExportSyntax::DefaultDeclaration => lsproto::ImportKind::Default,
        ExportSyntax::Named | ExportSyntax::Modifier | ExportSyntax::Star | ExportSyntax::CommonJSExportsProperty => {
            if export.syntax == ExportSyntax::Named && export.export_name == ast::InternalSymbolNameDefault {
                return lsproto::ImportKind::Default;
            }
            lsproto::ImportKind::Named
        }
        ExportSyntax::Equals | ExportSyntax::CommonJSModuleExports | ExportSyntax::UMD => {
            // export.Syntax will be ExportSyntaxEquals for named exports/properties of an export='s target.
            if export.export_name != ast::InternalSymbolNameExportEquals {
                return lsproto::ImportKind::Named;
            }
            // !!! cache this?
            for &statement in importing_file.statements.nodes() {
                // `import foo` parses as an ImportEqualsDeclaration even though it could be an ImportDeclaration
                if ast::is_import_equals_declaration(statement) && !ast::node_is_missing(Some(statement.as_import_equals_declaration().module_reference)) {
                    return lsproto::ImportKind::CommonJS;
                }
            }
            // !!! this logic feels weird; we're basically trying to predict if shouldUseRequire is going to
            //     be true. The meaning of "default import" is different depending on whether we write it as
            //     a require or an es6 import. The latter, compiled to CJS, has interop built in that will
            //     avoid accessing .default, but if we write a require directly and call it a default import,
            //     we emit an unconditional .default access.
            if importing_file.external_module_indicator().is_some() || force_import_keyword || !ast::is_source_file_js(importing_file) {
                return lsproto::ImportKind::Default;
            }
            lsproto::ImportKind::CommonJS
        }
        _ => panic!("unhandled export syntax kind: {}", export.syntax.string()),
    }
}

// fix.go:838
#[derive(Clone)]
pub(crate) struct existingImport {
    pub(crate) node: P<Node>,
    pub(crate) module_specifier: String,
    pub(crate) index: usize,
}

impl View {
    // fix.go:844
    fn get_existing_imports(&self) -> Rc<FxHashMap<ModuleID, Vec<existingImport>>> {
        self.existing_imports_cell()
            .get_or_init(|| {
                let mut result: FxHashMap<ModuleID, Vec<existingImport>> = FxHashMap::default();

                for (i, &module_specifier) in self.importing_file.imports().iter().enumerate() {
                    let Some(node) = ast::try_get_import_from_module_specifier(module_specifier) else {
                        panic!("error: did not expect node kind {:?}", module_specifier.kind());
                    };
                    if ast::is_variable_declaration_initialized_to_require(node.parent().unwrap()) {
                        if let Some(module_symbol) = self.checker().resolve_external_module_name_exported(module_specifier, None /*importAttributesType*/) {
                            if let Some((module_id, _)) = try_get_module_id_and_file_name_of_module_symbol(module_symbol) {
                                result.entry(module_id).or_default().push(existingImport {
                                    node: node.parent().unwrap(),
                                    module_specifier: module_specifier.text().to_string(),
                                    index: i,
                                });
                            }
                        }
                    } else if node.kind() == Kind::ImportDeclaration || node.kind() == Kind::ImportEqualsDeclaration || node.kind() == Kind::JSDocImportTag {
                        if let Some(module_symbol) = self.checker().get_symbol_at_location_exported(module_specifier) {
                            if let Some((module_id, _)) = try_get_module_id_and_file_name_of_module_symbol(module_symbol) {
                                result.entry(module_id).or_default().push(existingImport {
                                    node,
                                    module_specifier: module_specifier.text().to_string(),
                                    index: i,
                                });
                            }
                        }
                    }
                }
                Rc::new(result)
            })
            .clone()
    }

    // fix.go:876
    fn should_use_require(&self) -> bool {
        *self.should_use_require_cell().get_or_init(|| self.compute_should_use_require())
    }
}

// fix.go:888
// fileSyntaxKind represents the detected module syntax of a source file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum fileSyntaxKind {
    Ambiguous,
    ESM,
    CJS,
}

// fix.go:900
// detectSyntax returns whether a source file has unambiguous ESM or CJS syntax.
// When moduleDetection is "force", ExternalModuleIndicator may be set to the
// source file node itself rather than a genuine syntax indicator, so we fall back
// to inspecting the file's Imports() to find actual import/export declarations.
fn detect_syntax(file: P<SourceFile>, options: &CompilerOptions) -> fileSyntaxKind {
    let (has_esm, has_cjs) = detect_syntax_indicators(file, options);
    if has_cjs && !has_esm {
        fileSyntaxKind::CJS
    } else if has_esm && !has_cjs {
        fileSyntaxKind::ESM
    } else {
        fileSyntaxKind::Ambiguous
    }
}

// fix.go:916
// detectSyntaxIndicators checks whether a source file contains genuine ESM
// and/or CJS syntax. Under moduleDetection "force", the cached
// ExternalModuleIndicator may be the source file itself rather than a real
// statement, so we look at Imports() for actual import/export declarations.
fn detect_syntax_indicators(file: P<SourceFile>, options: &CompilerOptions) -> (bool, bool) {
    let has_cjs = file.common_js_module_indicator().is_some();
    if options.get_emit_module_detection_kind() != ModuleDetectionKind::Force {
        // ExternalModuleIndicator is reliable when moduleDetection is not "force"
        let has_esm = file.external_module_indicator().is_some();
        return (has_esm, has_cjs);
    }
    // Under moduleDetection "force", ExternalModuleIndicator is set to
    // file.AsNode() when there is no genuine ESM syntax, so only trust it
    // when it points to a real statement node.
    if let Some(indicator) = file.external_module_indicator() {
        if indicator != file.as_node() {
            return (true, has_cjs);
        }
    }
    // Fall back to scanning Imports() for actual import/export declarations
    // (not require() calls or dynamic imports).
    for &imp in file.imports() {
        if imp.flags().intersects(NodeFlags::Synthesized) {
            continue;
        }
        let Some(parent) = imp.parent() else {
            continue;
        };
        match parent.kind() {
            Kind::ImportDeclaration | Kind::JSImportDeclaration | Kind::ExportDeclaration => return (true, has_cjs),
            Kind::ExternalModuleReference => {
                // import x = require("...") — this is ESM-ish syntax
                return (true, has_cjs);
            }
            _ => {}
        }
    }
    (false, has_cjs)
}

impl View {
    // fix.go:949
    fn compute_should_use_require(&self) -> bool {
        // 1. TypeScript files don't use require variable declarations
        if !tspath::has_js_file_extension(self.importing_file.file_name()) {
            return false;
        }

        // 2. If the current source file is unambiguously CJS or ESM, go with that
        match detect_syntax(self.importing_file, &self.program.options()) {
            fileSyntaxKind::CJS => return true,
            fileSyntaxKind::ESM => return false,
            fileSyntaxKind::Ambiguous => {}
        }

        // 3. Use the implied node format to determine CJS vs ESM
        //    TODO: consider removing `impliedNodeFormatForEmit`
        match self.program.get_implied_node_format_for_emit(self.importing_file) {
            ModuleKind::CommonJS => return true,
            ModuleKind::ESNext => return false,
            _ => {}
        }

        // 4. If there's a tsconfig/jsconfig, use its module setting
        if !self.program.options().config_file_path.is_empty() {
            return self.program.options().get_emit_module_kind() < ModuleKind::ES2015;
        }

        // 5. Match the first other JS file in the program that's unambiguously CJS or ESM
        for &other_file in self.program.get_source_files() {
            if other_file == self.importing_file || !ast::is_source_file_js(other_file) || self.program.is_source_file_from_external_library(other_file) {
                continue;
            }
            match detect_syntax(other_file, &self.program.options()) {
                fileSyntaxKind::CJS => return true,
                fileSyntaxKind::ESM => return false,
                fileSyntaxKind::Ambiguous => {}
            }
        }

        // 6. Literally nothing to go on
        true
    }
}

// fix.go:996
fn needs_type_only(add_as_type_only: lsproto::AddAsTypeOnly) -> bool {
    add_as_type_only == lsproto::AddAsTypeOnly::Required
}

// fix.go:1000
fn should_use_type_only(add_as_type_only: lsproto::AddAsTypeOnly, preferences: &UserPreferences) -> bool {
    needs_type_only(add_as_type_only) || add_as_type_only != lsproto::AddAsTypeOnly::NotAllowed && preferences.prefer_type_only_auto_imports.is_true()
}

impl View {
    // fix.go:1008
    // CompareFixesForSorting returns negative if `a` is better than `b`.
    // Sorting with this comparator will place the best fix first.
    // After rank sorting, fixes will be sorted by arbitrary but stable criteria
    // to ensure a deterministic order.
    pub fn compare_fixes_for_sorting(&self, a: &Fix, b: &Fix) -> i32 {
        let res = self.compare_fixes_for_ranking(a, b);
        if res != 0 {
            return res;
        }
        self.compare_module_specifiers_for_sorting(a, b)
    }

    // fix.go:1018
    // CompareFixesForRanking returns negative if `a` is better than `b`.
    // Sorting with this comparator will place the best fix first.
    // Fixes of equal desirability will be considered equal.
    pub fn compare_fixes_for_ranking(&self, a: &Fix, b: &Fix) -> i32 {
        let res = compare_fix_kinds(a.kind, b.kind);
        if res != 0 {
            return res;
        }
        self.compare_module_specifiers_for_ranking(a, b)
    }
}

// fix.go:1025
fn compare_fix_kinds(a: lsproto::AutoImportFixKind, b: lsproto::AutoImportFixKind) -> i32 {
    a.0 - b.0
}

impl View {
    // fix.go:1029
    fn compare_module_specifiers_for_ranking(&self, a: &Fix, b: &Fix) -> i32 {
        let comparison = compare_module_specifier_relativity(a, b, &self.preferences);
        if comparison != 0 {
            return comparison;
        }
        if a.module_specifier_kind == ResultKind::Ambient && b.module_specifier_kind == ResultKind::Ambient {
            let comparison = self.compare_node_core_module_specifiers(&a.module_specifier, &b.module_specifier);
            if comparison != 0 {
                return comparison;
            }
        }
        if a.module_specifier_kind == ResultKind::Relative && b.module_specifier_kind == ResultKind::Relative {
            let comparison = tsrs_core::compare_booleans(
                is_fix_possibly_re_exporting_importing_file(a, self.importing_file.file_name()),
                is_fix_possibly_re_exporting_importing_file(b, self.importing_file.file_name()),
            );
            if comparison != 0 {
                return comparison;
            }
        }
        let comparison = tspath::compare_number_of_directory_separators(&a.module_specifier, &b.module_specifier);
        if comparison != 0 {
            return comparison;
        }
        0
    }

    // fix.go:1054
    fn compare_module_specifiers_for_sorting(&self, a: &Fix, b: &Fix) -> i32 {
        let res = self.compare_module_specifiers_for_ranking(a, b);
        if res != 0 {
            return res;
        }
        // Sort ./foo before ../foo for equal-length specifiers
        if a.module_specifier.starts_with("./") && !b.module_specifier.starts_with("./") {
            return -1;
        }
        if b.module_specifier.starts_with("./") && !a.module_specifier.starts_with("./") {
            return 1;
        }
        match a.module_specifier.cmp(&b.module_specifier) {
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Equal => {}
        }
        match a.import_kind.0.cmp(&b.import_kind.0) {
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Equal => {}
        }
        // !!! further tie-breakers? In practice this is only called on fixes with the same name
        0
    }

    // fix.go:1076
    fn compare_node_core_module_specifiers(&self, a: &str, b: &str) -> i32 {
        if a.starts_with("node:") && !b.starts_with("node:") {
            if self.should_use_uri_style_node_core_modules.is_true() {
                return -1;
            } else if self.should_use_uri_style_node_core_modules.is_false() {
                return 1;
            }
            return 0;
        }
        if b.starts_with("node:") && !a.starts_with("node:") {
            if self.should_use_uri_style_node_core_modules.is_true() {
                return 1;
            } else if self.should_use_uri_style_node_core_modules.is_false() {
                return -1;
            }
        }
        0
    }
}

// fix.go:1101
// This is a simple heuristic to try to avoid creating an import cycle with a barrel re-export.
// E.g., do not `import { Foo } from ".."` when you could `import { Foo } from "../Foo"`.
// This can produce false positives or negatives if re-exports cross into sibling directories
// (e.g. `export * from "../whatever"`) or are not named "index". Technically this should do
// a tspath.Path comparison, but it's not worth it to run a heuristic in such a hot path.
fn is_fix_possibly_re_exporting_importing_file(fix: &Fix, importing_file_name: &str) -> bool {
    if fix.is_re_export && is_index_file_name(&fix.module_file_name) {
        let re_export_dir = tspath::get_directory_path(&fix.module_file_name);
        return importing_file_name.starts_with(&tspath::ensure_trailing_directory_separator(&re_export_dir));
    }
    false
}

// fix.go:1109
fn is_index_file_name(file_name: &str) -> bool {
    let Some(last_slash) = file_name.rfind('/') else {
        return false;
    };
    if file_name.len() <= last_slash + 1 {
        return false;
    }
    matches!(&file_name[last_slash + 1..], "index.js" | "index.jsx" | "index.d.ts" | "index.ts" | "index.tsx")
}

// fix.go:1123
fn promote_from_type_only(
    changes: &mut change::Tracker,
    alias_declaration: P<Node>,
    compiler_options: &CompilerOptions,
    source_file: P<SourceFile>,
    preferences: &UserPreferences,
) -> P<Node> {
    // See comment in `doAddExistingFix` on constant with the same name.
    let convert_existing_to_type_only = compiler_options.verbatim_module_syntax;

    match alias_declaration.kind() {
        Kind::ImportSpecifier => {
            let spec = alias_declaration.as_import_specifier();
            if spec.is_type_only {
                if let Some(spec_parent) = alias_declaration.parent().filter(|p| p.kind() == Kind::NamedImports) {
                    let named_imports_node = spec_parent.as_named_imports();
                    let elements = named_imports_node.elements.nodes();
                    if elements.len() > 1 {
                        // Create a synthetic specifier with isTypeOnly=false to compute sorted position
                        let property_name = spec.property_name().map(|p| changes.node_factory.new_identifier(p.text()));
                        let new_specifier = changes.node_factory.new_import_specifier(
                            false, // isTypeOnly = false
                            property_name,
                            changes.node_factory.new_identifier(spec.name().text()),
                        );
                        let (specifier_comparer, _) = lsutil::get_named_import_specifier_comparer_with_detection(
                            spec_parent.parent().unwrap().parent().unwrap(), // ImportDeclaration
                            Some(source_file),
                            preferences,
                        );
                        let insertion_index = lsutil::get_import_specifier_insertion_index(elements, new_specifier, &*specifier_comparer);
                        let current_index = elements.iter().position(|&e| e == alias_declaration).map_or(-1, |i| i as i64);
                        if insertion_index as i64 != current_index {
                            changes.delete(source_file, alias_declaration);
                            changes.insert_import_specifier_at_index(source_file, new_specifier, spec_parent, insertion_index);
                            return alias_declaration;
                        }
                    }
                    // If no re-sorting needed, just remove the 'type' keyword
                    let first_token = lsutil::get_first_token(alias_declaration, source_file).unwrap();
                    let type_keyword_pos = scanner::get_token_pos_of_node(first_token, source_file, false);
                    let target_node = spec.property_name().unwrap_or_else(|| spec.name());
                    let target_pos = scanner::get_token_pos_of_node(target_node, source_file, false);
                    changes.delete_range(source_file, TextRange::new(type_keyword_pos, target_pos));
                }
                alias_declaration
            } else {
                // The parent import clause is type-only
                let Some(spec_parent) = alias_declaration.parent().filter(|p| p.kind() == Kind::NamedImports) else {
                    panic!("ImportSpecifier parent must be NamedImports");
                };
                let Some(clause) = spec_parent.parent().filter(|p| p.kind() == Kind::ImportClause) else {
                    panic!("NamedImports parent must be ImportClause");
                };
                promote_import_clause(changes, clause, compiler_options, source_file, preferences, convert_existing_to_type_only, Some(alias_declaration));
                clause
            }
        }

        Kind::ImportClause => {
            promote_import_clause(changes, alias_declaration, compiler_options, source_file, preferences, convert_existing_to_type_only, Some(alias_declaration));
            alias_declaration
        }

        Kind::NamespaceImport => {
            // Promote the parent import clause
            let Some(clause) = alias_declaration.parent().filter(|p| p.kind() == Kind::ImportClause) else {
                panic!("NamespaceImport parent must be ImportClause");
            };
            promote_import_clause(changes, clause, compiler_options, source_file, preferences, convert_existing_to_type_only, Some(alias_declaration));
            clause
        }

        Kind::ImportEqualsDeclaration => {
            // Remove the 'type' keyword (which is the second token: 'import' 'type' name '=' ...)
            // The type keyword is after 'import' and before the name
            let mut scan = scanner::get_scanner_for_source_file(source_file, alias_declaration.pos());
            // Skip 'import' keyword to get to 'type'
            scan.scan();
            delete_type_keyword(changes, source_file, scan.token_start());
            alias_declaration
        }
        _ => panic!("Unexpected alias declaration kind: {:?}", alias_declaration.kind()),
    }
}

// fix.go:1214
// promoteImportClause removes the type keyword from an import clause
fn promote_import_clause(
    changes: &mut change::Tracker,
    import_clause_node: P<Node>,
    compiler_options: &CompilerOptions,
    source_file: P<SourceFile>,
    preferences: &UserPreferences,
    convert_existing_to_type_only: Tristate,
    alias_declaration: Option<P<Node>>,
) {
    let import_clause = import_clause_node.as_import_clause();
    // Delete the 'type' keyword
    if import_clause.phase_modifier() == Kind::TypeKeyword {
        delete_type_keyword(changes, source_file, import_clause_node.pos());
    }

    // Handle .ts extension conversion to .js if necessary
    if compiler_options.allow_importing_ts_extensions.is_false() {
        let module_specifier = tsrs_checker::try_get_module_specifier_from_declaration(import_clause_node.parent().unwrap());
        if module_specifier.is_some() {
            // Note: We can't check ResolvedUsingTsExtension without program, so we'll skip this optimization
            // The fix will still work, just might not change .ts to .js extensions in all cases
        }
    }

    // Handle verbatimModuleSyntax conversion
    // If convertExistingToTypeOnly is true, we need to add 'type' to other specifiers
    // in the same import declaration
    if convert_existing_to_type_only.is_true() {
        if let Some(named_imports) = import_clause.named_bindings.filter(|n| n.kind() == Kind::NamedImports) {
            let named_imports_data = named_imports.as_named_imports();
            if named_imports_data.elements.nodes().len() > 1 {
                // Check if the list is sorted and if we need to reorder
                let (_, is_sorted) = lsutil::get_named_import_specifier_comparer_with_detection(import_clause_node.parent().unwrap(), Some(source_file), preferences);

                // If the alias declaration is an ImportSpecifier and the list is sorted,
                // move it to index 0 (since it will be the only non-type-only import)
                if !is_sorted.is_false() && // isSorted !== false
                    alias_declaration.is_some_and(|a| a.kind() == Kind::ImportSpecifier)
                {
                    let alias_declaration = alias_declaration.unwrap();
                    // Find the index of the alias declaration
                    let alias_index = named_imports_data.elements.nodes().iter().position(|&e| e == alias_declaration);
                    // If not already at index 0, move it there
                    if alias_index.is_some_and(|i| i > 0) {
                        // Delete the specifier from its current position
                        changes.delete(source_file, alias_declaration);
                        // Insert it at index 0
                        changes.insert_import_specifier_at_index(source_file, alias_declaration, named_imports, 0);
                    }
                }

                // Add 'type' keyword to all other import specifiers that aren't already type-only
                for &element in named_imports_data.elements.nodes() {
                    let spec = element.as_import_specifier();
                    // Skip the specifier being promoted (if aliasDeclaration is an ImportSpecifier)
                    if let Some(alias_declaration) = alias_declaration {
                        if alias_declaration.kind() == Kind::ImportSpecifier && element == alias_declaration {
                            continue;
                        }
                    }
                    // Skip if already type-only
                    if !spec.is_type_only {
                        changes.insert_modifier_before(source_file, Kind::TypeKeyword, element);
                    }
                }
            }
        }
    }
}

// fix.go:1293
// deleteTypeKeyword deletes the 'type' keyword token starting at the given position,
// including any trailing whitespace.
fn delete_type_keyword(changes: &mut change::Tracker, source_file: P<SourceFile>, start_pos: i32) {
    let scan = scanner::get_scanner_for_source_file(source_file, start_pos);
    if scan.token() != Kind::TypeKeyword {
        return;
    }
    let type_start = scan.token_start();
    let mut type_end = scan.token_end();
    // Skip trailing whitespace
    let text = source_file.text().as_bytes();
    while (type_end as usize) < text.len() && (text[type_end as usize] == b' ' || text[type_end as usize] == b'\t') {
        type_end += 1;
    }
    changes.delete_range(source_file, TextRange::new(type_start, type_end));
}

// fix.go:1309
fn get_module_specifier_text(promoted_declaration: P<Node>) -> String {
    if promoted_declaration.kind() == Kind::ImportEqualsDeclaration {
        let import_equals_declaration = promoted_declaration.as_import_equals_declaration();
        if ast::is_external_module_reference(import_equals_declaration.module_reference) {
            if let Some(expr) = import_equals_declaration.module_reference.expression() {
                if ast::is_string_literal_like(expr) {
                    return expr.text().to_string();
                }
                return scanner::get_text_of_node(expr);
            }
        }
        return scanner::get_text_of_node(import_equals_declaration.module_reference);
    }
    let module_specifier = promoted_declaration.parent().unwrap().module_specifier().unwrap();
    if ast::is_string_literal_like(module_specifier) {
        return module_specifier.text().to_string();
    }
    scanner::get_text_of_node(module_specifier)
}

// fix.go:1325
// returns `-1` if `a` is better than `b`
fn compare_module_specifier_relativity(a: &Fix, b: &Fix, preferences: &modulespecifiers::UserPreferences) -> i32 {
    match preferences.import_module_specifier_preference {
        ImportModuleSpecifierPreference::NonRelative | ImportModuleSpecifierPreference::ProjectRelative => {
            tsrs_core::compare_booleans(a.module_specifier_kind == ResultKind::Relative, b.module_specifier_kind == ResultKind::Relative)
        }
        _ => 0,
    }
}
