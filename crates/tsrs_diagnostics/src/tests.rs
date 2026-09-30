use rustc_hash::FxHashSet;

use crate as diagnostics;
use crate::generated::ALL_MESSAGES;
use crate::{format, format_strings, key_to_message, localize, Category, Key};

#[test]
fn message_count_matches_go() {
    // `grep -c '= &Message{' diagnostics_generated.go` at the pinned commit.
    assert_eq!(ALL_MESSAGES.len(), 2215);
    let keys: FxHashSet<&str> = ALL_MESSAGES.iter().map(|m| m.key().as_str()).collect();
    assert_eq!(keys.len(), ALL_MESSAGES.len());
    let codes: FxHashSet<i32> = ALL_MESSAGES.iter().map(|m| m.code()).collect();
    assert_eq!(codes.len(), ALL_MESSAGES.len());
}

#[test]
fn spot_checks() {
    let m = &diagnostics::Type_0_is_not_assignable_to_type_1;
    assert_eq!(m.code(), 2322);
    assert_eq!(m.category(), Category::Error);
    assert_eq!(m.key(), Key("Type_0_is_not_assignable_to_type_1_2322"));
    assert_eq!(m.text(), "Type '{0}' is not assignable to type '{1}'.");

    assert_eq!(diagnostics::X_0_expected.code(), 1005);
    assert_eq!(diagnostics::X_0_expected.key().as_str(), "_0_expected_1005");

    assert!(diagnostics::Call_signature_return_types_0_and_1_are_incompatible.elided_in_compatibility_pyramid());
    assert!(!diagnostics::Type_0_is_not_assignable_to_type_1.reports_unnecessary());
    assert_eq!(
        diagnostics::X_k_must_be_followed_by_a_capturing_group_name_enclosed_in_angle_brackets.text(),
        "'\\k' must be followed by a capturing group name enclosed in angle brackets."
    );
    assert_eq!(
        diagnostics::Module_declaration_names_may_only_use_or_quoted_strings.text(),
        "Module declaration names may only use ' or \" quoted strings."
    );

    assert!(ALL_MESSAGES.iter().any(|m| m.reports_unnecessary()));
    assert!(ALL_MESSAGES.iter().any(|m| m.reports_deprecated()));
    assert!(ALL_MESSAGES.iter().any(|m| m.category() == Category::Suggestion));
    assert!(ALL_MESSAGES.iter().any(|m| m.category() == Category::Message));
}

#[test]
fn category_names() {
    assert_eq!(Category::Error.name(), "error");
    assert_eq!(Category::Warning.name(), "warning");
    assert_eq!(Category::Suggestion.name(), "suggestion");
    assert_eq!(Category::Message.name(), "message");
    assert_eq!(Category::Error.to_string(), "CategoryError");
    assert_eq!(Category::Warning as i32, 0);
    assert_eq!(Category::Message as i32, 3);
}

#[test]
fn key_lookup() {
    let m = key_to_message("Identifier_expected_1003").unwrap();
    assert!(std::ptr::eq(m, &diagnostics::Identifier_expected));
    assert!(key_to_message("Removed_diagnostic_99999").is_none());
    assert_eq!(localize(None, Key("_0_expected_1005"), &[")".to_string()]), "')' expected.");
    assert_eq!(localize(None, Key("Identifier_expected_1003"), &[]), "Identifier expected.");
}

#[test]
fn formatting() {
    assert_eq!(diagnostics::Identifier_expected.format(&[]), "Identifier expected.");
    assert_eq!(diagnostics::X_0_expected.format(&[&")"]), "')' expected.");
    assert_eq!(
        diagnostics::The_parser_expected_to_find_a_1_to_match_the_0_token_here.format(&[&"{", &"}"]),
        "The parser expected to find a '}' to match the '{' token here."
    );
    assert_eq!(
        diagnostics::Type_0_is_not_assignable_to_type_1.localize(&[&"string", &42]),
        "Type 'string' is not assignable to type '42'."
    );
    // Substituted text is not re-scanned; repeated and out-of-order placeholders work.
    assert_eq!(format("{1}{0}{1}", &[&"{1}", &"b"]), "b{1}b");
    // Only `{digits}` is a placeholder.
    assert_eq!(format("{{0}} {x} {} {0", &[&"a"]), "{a} {x} {} {0");
    // No args: text is returned unchanged even with placeholders (Go early return).
    assert_eq!(format("'{0}'", &[]), "'{0}'");
    assert_eq!(format_strings("é{0}ü", &["ß".to_string()]), "éßü");
}

#[test]
#[should_panic(expected = "Invalid formatting placeholder")]
fn formatting_missing_arg_panics() {
    format("{0} {1}", &[&"a"]);
}

#[test]
fn ad_hoc_message() {
    let m = diagnostics::new_ad_hoc_message("hello {0}");
    assert_eq!(m.code(), -1);
    assert_eq!(m.category(), Category::Error);
    assert_eq!(m.key().as_str(), "-1");
    assert_eq!(m.format(&[&"world"]), "hello world");
}
