use serde::Deserialize;
use serde_json::Value;
use tsrs_ast::{self as ast, Kind, Node, OperatorPrecedence, OperatorPrecedenceFlags};
use tsrs_checker::{Checker, Type, is_tuple_type_exported};
use tsrs_core::{P, TextRange};

use crate::utils::{
    TypeOrValueSpecifier, is_builtin_promise_like, type_matches_some_specifier, union_parts,
    value_matches_some_specifier,
};
use crate::{RuleContext, RuleDefinition, RuleFix, RuleLabeledRange, RuleMessage, RuleSuggestion};

#[derive(Clone, Debug)]
struct Options {
    allow_for_known_safe_calls: Vec<TypeOrValueSpecifier>,
    allow_for_known_safe_promises: Vec<TypeOrValueSpecifier>,
    check_thenables: bool,
    ignore_iife: bool,
    ignore_void: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawOptions {
    allow_for_known_safe_calls: Option<Vec<TypeOrValueSpecifier>>,
    allow_for_known_safe_promises: Option<Vec<TypeOrValueSpecifier>>,
    check_thenables: Option<bool>,
    #[serde(rename = "ignoreIIFE")]
    ignore_iife: Option<bool>,
    ignore_void: Option<bool>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            allow_for_known_safe_calls: Vec::new(),
            allow_for_known_safe_promises: Vec::new(),
            check_thenables: false,
            ignore_iife: false,
            ignore_void: true,
        }
    }
}

#[derive(Clone, Copy)]
struct UnhandledPromise {
    node: P<Node>,
    type_: P<Type>,
    promise_array: bool,
    non_function_handler: Option<P<Node>>,
}

fn message(id: &str, description: &str, help: Option<&str>) -> RuleMessage {
    RuleMessage::new(id, description, help)
}

const BASE: &str = "Promises must be awaited, add await operator.";
const BASE_HELP: &str = "The promise must end with a call to .catch, or end with a call to .then with a rejection handler.";
const VOID: &str = "Promises must be awaited, add void operator to ignore.";
const VOID_HELP: &str = "The promise must end with a call to .catch, or end with a call to .then with a rejection handler, or be explicitly marked as ignored with the `void` operator.";
const HANDLER: &str = "A rejection handler that is not a function will be ignored.";

fn higher_than_unary(node: P<Node>) -> bool {
    let operator = if ast::is_binary_expression(node) {
        node.as_binary_expression().operator_token().kind()
    } else {
        Kind::Unknown
    };
    ast::get_operator_precedence(node.kind(), operator, OperatorPrecedenceFlags::None)
        > OperatorPrecedence::Unary
}

fn insert_before(ctx: &RuleContext<'_>, node: P<Node>, text: &str) -> RuleFix {
    let pos = ctx.trim_node_range(node).pos();
    RuleFix {
        text: text.to_string(),
        range: TextRange::new(pos, pos),
    }
}

fn insert_after(node: P<Node>, text: &str) -> RuleFix {
    RuleFix {
        text: text.to_string(),
        range: TextRange::new(node.end(), node.end()),
    }
}

fn add_await(ctx: &RuleContext<'_>, expression: P<Node>, statement: P<Node>) -> Vec<RuleFix> {
    if ast::is_void_expression(expression) {
        let range = tsrs_scanner::get_range_of_token_at_position(ctx.source_file, expression.pos());
        return vec![RuleFix {
            text: "await".to_string(),
            range,
        }];
    }
    if higher_than_unary(statement.as_expression_statement().expression()) {
        return vec![insert_before(ctx, statement, "await ")];
    }
    vec![
        insert_before(ctx, statement, "await ("),
        insert_after(expression, ")"),
    ]
}

fn is_function_parameter(
    checker: &mut Checker,
    parameter: P<tsrs_ast::Symbol>,
    node: P<Node>,
) -> bool {
    let Some(t) = checker.get_type_of_symbol_at_location(parameter, Some(node)) else {
        return false;
    };
    let apparent = checker.get_apparent_type(t);
    union_parts(apparent)
        .iter()
        .copied()
        .any(|part| !checker.get_call_signatures(part).is_empty())
}

fn has_matching_signature(
    checker: &mut Checker,
    t: P<Type>,
    mut matcher: impl FnMut(&mut Checker, P<tsrs_checker::Signature>) -> bool,
) -> bool {
    for part in union_parts(t) {
        for &signature in checker.get_call_signatures(part) {
            if matcher(checker, signature) {
                return true;
            }
        }
    }
    false
}

fn is_promise_like(
    ctx: &mut RuleContext<'_>,
    opts: &Options,
    node: P<Node>,
    t: Option<P<Type>>,
) -> bool {
    let t = t.unwrap_or_else(|| ctx.checker.get_type_at_location(node));
    if type_matches_some_specifier(t, &opts.allow_for_known_safe_promises, ctx.program) {
        return false;
    }
    let apparent = ctx.checker.get_apparent_type(t);
    if union_parts(apparent)
        .iter()
        .copied()
        .any(|part| is_builtin_promise_like(ctx.program, ctx.checker, part))
    {
        return true;
    }
    if !opts.check_thenables {
        return false;
    }
    for part in union_parts(apparent) {
        let Some(then) = ctx.checker.get_property_of_type(part, "then") else {
            continue;
        };
        let Some(then_type) = ctx.checker.get_type_of_symbol_at_location(then, Some(node)) else {
            continue;
        };
        if has_matching_signature(ctx.checker, then_type, |checker, signature| {
            let parameters = signature.parameters();
            parameters.len() >= 2
                && is_function_parameter(checker, parameters[0], node)
                && is_function_parameter(checker, parameters[1], node)
        }) {
            return true;
        }
    }
    false
}

fn is_promise_array(ctx: &mut RuleContext<'_>, opts: &Options, node: P<Node>, t: P<Type>) -> bool {
    for part in union_parts(t) {
        let apparent = ctx.checker.get_apparent_type(part);
        if ctx.checker.is_array_type(apparent) {
            if ctx
                .checker
                .get_type_arguments(apparent)
                .first()
                .is_some_and(|&element| is_promise_like(ctx, opts, node, Some(element)))
            {
                return true;
            }
        }
        if is_tuple_type_exported(apparent)
            && ctx
                .checker
                .get_type_arguments(apparent)
                .iter()
                .copied()
                .any(|element| is_promise_like(ctx, opts, node, Some(element)))
        {
            return true;
        }
    }
    false
}

fn valid_rejection_handler(checker: &mut Checker, node: P<Node>) -> bool {
    let t = checker.get_type_at_location(node);
    !checker.get_call_signatures(t).is_empty()
}

fn known_argument_at(arguments: &[P<Node>], index: usize) -> bool {
    arguments.len() > index
        && !arguments[..=index]
            .iter()
            .copied()
            .any(ast::is_spread_element)
}

fn is_unhandled(
    ctx: &mut RuleContext<'_>,
    opts: &Options,
    node: P<Node>,
) -> Option<UnhandledPromise> {
    if ast::is_assignment_expression(node, false) {
        return None;
    }
    if ast::is_comma_expression(node) {
        let expression = node.as_binary_expression();
        return is_unhandled(ctx, opts, expression.left())
            .or_else(|| is_unhandled(ctx, opts, expression.right()));
    }
    if !opts.ignore_void && ast::is_void_expression(node) {
        return is_unhandled(ctx, opts, node.expression().unwrap());
    }
    let t = ctx.checker.get_type_at_location(node);
    if is_promise_array(ctx, opts, node, t) {
        return Some(UnhandledPromise {
            node,
            type_: t,
            promise_array: true,
            non_function_handler: None,
        });
    }
    if ast::is_await_expression(node) {
        return None;
    }
    if !is_promise_like(ctx, opts, node, Some(t)) {
        return None;
    }
    if ast::is_call_expression(node) {
        let call = node.as_call_expression();
        let callee = call.expression();
        if ast::is_access_expression(callee) {
            let (method, _) = ctx.checker.get_accessed_property_name(callee);
            let arguments = call.arguments().nodes();
            if method == "catch" && !arguments.is_empty() {
                if !known_argument_at(arguments, 0) {
                    return Some(UnhandledPromise {
                        node,
                        type_: t,
                        promise_array: false,
                        non_function_handler: None,
                    });
                }
                if valid_rejection_handler(ctx.checker, arguments[0]) {
                    return None;
                }
                return Some(UnhandledPromise {
                    node,
                    type_: t,
                    promise_array: false,
                    non_function_handler: Some(arguments[0]),
                });
            }
            if method == "then" && arguments.len() >= 2 {
                if !known_argument_at(arguments, 1) {
                    return Some(UnhandledPromise {
                        node,
                        type_: t,
                        promise_array: false,
                        non_function_handler: None,
                    });
                }
                if valid_rejection_handler(ctx.checker, arguments[1]) {
                    return None;
                }
                return Some(UnhandledPromise {
                    node,
                    type_: t,
                    promise_array: false,
                    non_function_handler: Some(arguments[1]),
                });
            }
            if method == "finally" {
                let mut result = is_unhandled(ctx, opts, callee.expression().unwrap())?;
                result.node = node;
                result.type_ = t;
                return Some(result);
            }
        }
        return Some(UnhandledPromise {
            node,
            type_: t,
            promise_array: false,
            non_function_handler: None,
        });
    }
    if ast::is_conditional_expression(node) {
        let expression = node.as_conditional_expression();
        return is_unhandled(ctx, opts, expression.when_false())
            .or_else(|| is_unhandled(ctx, opts, expression.when_true()));
    }
    if ast::is_logical_or_coalescing_binary_expression(node) {
        let expression = node.as_binary_expression();
        return is_unhandled(ctx, opts, expression.left())
            .or_else(|| is_unhandled(ctx, opts, expression.right()));
    }
    Some(UnhandledPromise {
        node,
        type_: t,
        promise_array: false,
        non_function_handler: None,
    })
}

fn known_safe_call(ctx: &mut RuleContext<'_>, opts: &Options, node: P<Node>) -> bool {
    if opts.allow_for_known_safe_calls.is_empty() || !ast::is_call_expression(node) {
        return false;
    }
    let callee = node.as_call_expression().expression();
    if value_matches_some_specifier(
        callee,
        &opts.allow_for_known_safe_calls,
        ctx.program,
        ctx.checker,
    ) {
        return true;
    }
    let t = ctx.checker.get_type_at_location(callee);
    type_matches_some_specifier(t, &opts.allow_for_known_safe_calls, ctx.program)
}

fn labels(ctx: &mut RuleContext<'_>, result: UnhandledPromise) -> Vec<RuleLabeledRange> {
    let range = ctx.trim_node_range(result.node);
    let prefix = if result.promise_array {
        "This array contains Promises and"
    } else {
        "This unhandled promise-like value"
    };
    let mut labels = vec![RuleLabeledRange {
        label: format!(
            "{} has type `{}`.",
            prefix,
            ctx.checker.type_to_string_exported(result.type_)
        ),
        range,
    }];
    if let Some(handler) = result.non_function_handler {
        let handler_type = ctx.checker.get_type_at_location(handler);
        labels.push(RuleLabeledRange {
            label: format!(
                "This rejection handler has type `{}`, which is not callable.",
                ctx.checker.type_to_string_exported(handler_type)
            ),
            range: ctx.trim_node_range(handler),
        });
    }
    labels
}

fn check_statement(ctx: &mut RuleContext<'_>, opts: &Options, statement: P<Node>) {
    let statement_expression = statement.as_expression_statement().expression();
    if opts.ignore_iife && ast::is_call_expression(statement_expression) {
        let callee = ast::skip_parentheses(statement_expression.as_call_expression().expression());
        if ast::is_arrow_function(callee) || ast::is_function_expression(callee) {
            return;
        }
    }
    let expression = ast::skip_parentheses(statement_expression);
    if known_safe_call(ctx, opts, expression) {
        return;
    }
    let Some(result) = is_unhandled(ctx, opts, expression) else {
        return;
    };
    let range = ctx.trim_node_range(result.node);
    let labeled_ranges = labels(ctx, result);
    if result.promise_array {
        let diagnostic = if opts.ignore_void {
            message(
                "floatingPromiseArrayVoid",
                "An array of Promises may be unintentional.",
                Some(
                    "Consider handling the promises' fulfillment or rejection with Promise.all or similar, or explicitly marking the expression as ignored with the `void` operator.",
                ),
            )
        } else {
            message(
                "floatingPromiseArray",
                "An array of Promises may be unintentional.",
                Some(
                    "Consider handling the promises' fulfillment or rejection with Promise.all or similar.",
                ),
            )
        };
        ctx.report(range, diagnostic, labeled_ranges);
        return;
    }
    let invalid_handler = result.non_function_handler.is_some();
    let diagnostic = match (opts.ignore_void, invalid_handler) {
        (true, true) => message(
            "floatingUselessRejectionHandlerVoid",
            VOID,
            Some(&format!("{VOID_HELP} {HANDLER}")),
        ),
        (true, false) => message("floatingVoid", VOID, Some(VOID_HELP)),
        (false, true) => message(
            "floatingUselessRejectionHandler",
            BASE,
            Some(&format!("{BASE_HELP} {HANDLER}")),
        ),
        (false, false) => message("floating", BASE, Some(BASE_HELP)),
    };
    let suggestions = if ctx.fixes.fix_suggestions {
        let mut suggestions = Vec::new();
        if opts.ignore_void {
            let fixes = if higher_than_unary(statement_expression) {
                vec![insert_before(ctx, statement, "void ")]
            } else {
                vec![
                    insert_before(ctx, statement, "void ("),
                    insert_after(expression, ")"),
                ]
            };
            suggestions.push(RuleSuggestion {
                message: message("floatingFixVoid", "Add void operator to ignore.", None),
                fixes,
            });
        }
        suggestions.push(RuleSuggestion {
            message: message("floatingFixAwait", "Add await operator.", None),
            fixes: add_await(ctx, expression, statement),
        });
        suggestions
    } else {
        Vec::new()
    };
    (ctx.on_diagnostic)(crate::RuleDiagnostic {
        range,
        rule_name: ctx.rule_name,
        message: diagnostic,
        fixes: Vec::new(),
        suggestions,
        source_file: ctx.source_file,
        labeled_ranges,
    });
}

fn run(ctx: &mut RuleContext<'_>, value: &Value) -> Result<u64, String> {
    let opts = if value.is_null() {
        Options::default()
    } else {
        let raw: RawOptions = serde_json::from_value(value.clone())
            .map_err(|error| format!("no-floating-promises: {error}"))?;
        Options {
            allow_for_known_safe_calls: raw.allow_for_known_safe_calls.unwrap_or_default(),
            allow_for_known_safe_promises: raw.allow_for_known_safe_promises.unwrap_or_default(),
            check_thenables: raw.check_thenables.unwrap_or(false),
            ignore_iife: raw.ignore_iife.unwrap_or(false),
            ignore_void: raw.ignore_void.unwrap_or(true),
        }
    };
    let nodes = ctx
        .source_file
        .lint_nodes
        .get()
        .expect("lint nodes must be enabled before binding");
    let mut calls = 1;
    for &node in nodes.expression_statements.get() {
        check_statement(ctx, &opts, node);
        calls += 1;
    }
    Ok(calls)
}

pub static NO_FLOATING_PROMISES: RuleDefinition = RuleDefinition {
    name: "no-floating-promises",
    run,
};

#[cfg(test)]
mod test {
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};
    use tsrs_vfs::{FS, bundled, vfstest};

    use crate::{ConfiguredRule, Fixes, RunLinterOptions, TypeErrors, Workload, run_linter};

    use super::NO_FLOATING_PROMISES;

    fn lint(code: &str, options: Value, suggestions: bool) -> Vec<crate::RuleDiagnostic> {
        lint_files(&[("/file.ts", code)], options, suggestions)
    }

    fn lint_files(
        files: &[(&str, &str)],
        options: Value,
        suggestions: bool,
    ) -> Vec<crate::RuleDiagnostic> {
        let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(
            files.iter().copied(),
            true,
        )));
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let output = Arc::clone(&diagnostics);
        let run_options = RunLinterOptions {
            current_directory: "/".to_string(),
            workload: Workload {
                programs: Default::default(),
                unmatched_files: vec!["/file.ts".to_string()],
            },
            fs,
            get_rules_for_file: Arc::new(move |_| {
                vec![ConfiguredRule {
                    definition: &NO_FLOATING_PROMISES,
                    options: options.clone(),
                }]
            }),
            on_rule_diagnostic: Arc::new(move |diagnostic| output.lock().unwrap().push(diagnostic)),
            on_internal_diagnostic: Arc::new(|diagnostic| {
                panic!("unexpected internal diagnostic: {diagnostic:?}")
            }),
            fixes: Fixes {
                fix: false,
                fix_suggestions: suggestions,
            },
            type_errors: TypeErrors::default(),
            suppress_program_diagnostics: false,
            timings: false,
        };
        run_linter(&run_options).unwrap();
        diagnostics.lock().unwrap().clone()
    }

    fn ids(code: &str, options: Value) -> Vec<String> {
        lint(code, options, false)
            .into_iter()
            .map(|diagnostic| diagnostic.message.id)
            .collect()
    }

    #[test]
    fn handles_await_catch_then_void_and_assignments() {
        let code = r#"
async function test() {
  await Promise.resolve();
  Promise.resolve().catch(() => {});
  Promise.resolve().then(() => {}, () => {});
  void Promise.resolve();
  const value = Promise.resolve();
  return Promise.resolve();
}
"#;
        assert!(ids(code, Value::Null).is_empty());
    }

    #[test]
    fn reports_floating_promises_and_arrays() {
        let code = r#"
Promise.resolve();
declare const promises: Array<Promise<void>>;
promises;
Promise.reject().catch(undefined);
"#;
        assert_eq!(
            ids(code, Value::Null),
            [
                "floatingVoid",
                "floatingPromiseArrayVoid",
                "floatingUselessRejectionHandlerVoid"
            ]
        );
        assert_eq!(
            ids(code, json!({ "ignoreVoid": false })),
            [
                "floating",
                "floatingPromiseArray",
                "floatingUselessRejectionHandler"
            ]
        );
    }

    #[test]
    fn recognizes_derived_union_array_and_tuple_promises() {
        let code = r#"
declare class DerivedPromise extends Promise<void> {}
declare const derived: DerivedPromise;
declare const union: Promise<void> | number;
declare const array: Promise<void>[];
declare const tuple: [number, Promise<void>];
derived;
union;
array;
tuple;
"#;
        assert_eq!(
            ids(code, Value::Null),
            [
                "floatingVoid",
                "floatingVoid",
                "floatingPromiseArrayVoid",
                "floatingPromiseArrayVoid"
            ]
        );
    }

    #[test]
    fn checks_each_promise_branch() {
        let code = r#"
(Promise.resolve(), 1);
true ? 1 : Promise.resolve();
false || Promise.resolve();
"#;
        assert_eq!(
            ids(code, Value::Null),
            ["floatingVoid", "floatingVoid", "floatingVoid"]
        );
    }

    #[test]
    fn thenables_are_opt_in() {
        let code = r#"
declare const thenable: {
  then(onfulfilled: () => void, onrejected: () => void): unknown;
};
thenable;
"#;
        assert!(ids(code, Value::Null).is_empty());
        assert_eq!(
            ids(code, json!({ "checkThenables": true })),
            ["floatingVoid"]
        );
    }

    #[test]
    fn supports_safe_promise_and_call_specifiers() {
        let code = r#"
type SafePromise = Promise<void> & { readonly safe: unique symbol };
declare const safe: SafePromise;
declare function fire(): Promise<void>;
safe;
fire();
"#;
        let options = json!({
            "allowForKnownSafePromises": [{ "from": "file", "name": "SafePromise" }],
            "allowForKnownSafeCalls": [{ "from": "file", "name": "fire", "path": "/file.ts" }]
        });
        assert!(ids(code, options).is_empty());
    }

    #[test]
    fn supports_safe_call_specifiers_from_packages() {
        let files = [
            (
                "/file.ts",
                "import { fire } from 'safe-package';\nfire();\n",
            ),
            (
                "/node_modules/safe-package/package.json",
                r#"{ "name": "safe-package", "types": "index.d.ts" }"#,
            ),
            (
                "/node_modules/safe-package/index.d.ts",
                "export declare function fire(): Promise<void>;",
            ),
        ];
        assert_eq!(
            lint_files(&files, Value::Null, false)
                .into_iter()
                .map(|diagnostic| diagnostic.message.id)
                .collect::<Vec<_>>(),
            ["floatingVoid"]
        );
        let options = json!({ "allowForKnownSafeCalls": [{ "from": "package", "name": "fire", "package": "safe-package" }] });
        assert!(lint_files(&files, options, false).is_empty());
    }

    #[test]
    fn computed_bindings_do_not_match_safe_call_declaration_sources() {
        let code = r#"
declare const key: "fire";
declare const api: { fire: () => Promise<void> };
const { [key]: trusted } = api;
trusted();
"#;
        assert_eq!(
            ids(
                code,
                json!({ "allowForKnownSafeCalls": [{ "from": "file", "path": "/file.ts", "name": "trusted" }] }),
            ),
            ["floatingVoid"]
        );
        assert!(
            ids(
                code,
                json!({ "allowForKnownSafeCalls": [{ "from": "file", "path": "/file.ts", "name": "trusted" }, "trusted"] }),
            )
            .is_empty()
        );
    }

    #[test]
    fn safe_call_package_sources_stop_at_named_modules_but_skip_namespaces() {
        let files = [
            (
                "/file.ts",
                "/// <reference path='./safe.d.ts' />\nimport { InnerModule, InnerNamespace } from 'safe-package';\nInnerModule.fire();\nInnerNamespace.fire();\n",
            ),
            (
                "/safe.d.ts",
                r#"
declare module "safe-package" {
    export module InnerModule { function fire(): Promise<void>; }
    export namespace InnerNamespace { function fire(): Promise<void>; }
}
"#,
            ),
        ];
        let options = json!({ "allowForKnownSafeCalls": [{ "from": "package", "name": "fire", "package": "safe-package" }] });
        let diagnostics = lint_files(&files, options, false);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message.id, "floatingVoid");
        let range = diagnostics[0].range;
        assert_eq!(
            &files[0].1[range.pos() as usize..range.end() as usize],
            "InnerModule.fire()"
        );
    }

    #[test]
    fn safe_call_package_sources_fall_back_when_package_name_is_empty() {
        let files = [
            (
                "/file.ts",
                "import { fire } from 'safe-package';\nfire();\n",
            ),
            (
                "/node_modules/safe-package/package.json",
                r#"{ "name": "", "types": "index.d.ts" }"#,
            ),
            (
                "/node_modules/safe-package/index.d.ts",
                "export declare function fire(): Promise<void>;",
            ),
        ];
        assert_eq!(lint_files(&files, Value::Null, false).len(), 1);
        let options = json!({ "allowForKnownSafeCalls": [{ "from": "package", "name": "fire", "package": "safe-package" }] });
        assert!(lint_files(&files, options, false).is_empty());
    }

    #[test]
    fn ignores_iifes_when_configured() {
        let code = "(async () => {})();";
        assert_eq!(ids(code, Value::Null), ["floatingVoid"]);
        assert!(ids(code, json!({ "ignoreIIFE": true })).is_empty());
    }

    #[test]
    fn suggestions_match_the_headless_fix_contract() {
        let diagnostics = lint("Promise.resolve();", Value::Null, true);
        assert_eq!(diagnostics.len(), 1);
        let suggestions = &diagnostics[0].suggestions;
        assert_eq!(
            suggestions
                .iter()
                .map(|suggestion| suggestion.message.id.as_str())
                .collect::<Vec<_>>(),
            ["floatingFixVoid", "floatingFixAwait"]
        );
        assert_eq!(suggestions[0].fixes[0].text, "void ");
        assert_eq!(suggestions[1].fixes[0].text, "await ");
        assert_eq!(suggestions[0].fixes[0].range.pos(), 0);
        assert_eq!(suggestions[0].fixes[0].range.end(), 0);
    }
}
