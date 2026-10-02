use tsrs_core::{ScriptKind, Tristate, P};

use super::*;

fn parse_ts(text: &str) -> P<tsrs_ast::SourceFile> {
    crate::astnav::parse_for_test("/test.ts", text, ScriptKind::TS)
}

// utilities_test.go:19
#[test]
fn test_probably_uses_semicolons() {
    struct Test {
        name: &'static str,
        src: &'static str,
        want: bool,
    }
    let tests = [
        Test {
            name: "mixed semicolons and ASI favors semicolons when ratio exceeds one fifth",
            // First five observations: 2 with semicolon, 3 without. Real ratio 2/3 > 1/5.
            // Integer division bug compared against 1/5==0 and used with/without as ints,
            // so the old check was effectively (with/without) > 0, which failed here.
            src: "let a = 1;\nlet b = 2;\nlet c = 3\nlet d = 4\nlet e = 5\n",
            want: true,
        },
        Test { name: "consistent ASI with no semicolons", src: "let a = 1\nlet b = 2\nlet c = 3\n", want: false },
        Test { name: "consistent semicolons", src: "let a = 1;\nlet b = 2;\nlet c = 3;\n", want: true },
    ];

    for tt in tests {
        let file = parse_ts(tt.src);
        let got = probably_uses_semicolons(file);
        assert_eq!(got, tt.want, "{}: ProbablyUsesSemicolons() = {}, want {}", tt.name, got, tt.want);
    }
}

// utilities_test.go:70
#[test]
fn test_resolve_organize_imports_sort() {
    struct Test {
        name: &'static str,
        preferences: UserPreferences,
        want: OrganizeImportsSort,
    }
    let tests = [
        Test {
            name: "explicit sort wins",
            preferences: UserPreferences {
                organize_imports_sort: OrganizeImportsSort::Ordinal,
                organize_imports_collation: OrganizeImportsCollation::Unicode,
                organize_imports_ignore_case: Tristate::True,
                ..Default::default()
            },
            want: OrganizeImportsSort::Ordinal,
        },
        Test {
            name: "unicode case-sensitive maps to natural",
            preferences: UserPreferences {
                organize_imports_collation: OrganizeImportsCollation::Unicode,
                organize_imports_ignore_case: Tristate::False,
                ..Default::default()
            },
            want: OrganizeImportsSort::Natural,
        },
        Test {
            name: "unicode ignore case maps to natural ignore case",
            preferences: UserPreferences {
                organize_imports_collation: OrganizeImportsCollation::Unicode,
                organize_imports_ignore_case: Tristate::True,
                ..Default::default()
            },
            want: OrganizeImportsSort::NaturalIgnoreCase,
        },
        Test {
            name: "unicode unknown case sensitivity stays auto for detection",
            preferences: UserPreferences { organize_imports_collation: OrganizeImportsCollation::Unicode, ..Default::default() },
            want: OrganizeImportsSort::Auto,
        },
        Test {
            name: "ordinal ignore case maps to ordinal ignore case",
            preferences: UserPreferences { organize_imports_ignore_case: Tristate::True, ..Default::default() },
            want: OrganizeImportsSort::OrdinalIgnoreCase,
        },
        Test {
            name: "ordinal case sensitive maps to ordinal",
            preferences: UserPreferences { organize_imports_ignore_case: Tristate::False, ..Default::default() },
            want: OrganizeImportsSort::Ordinal,
        },
        Test { name: "unknown ordinal stays auto", preferences: UserPreferences::default(), want: OrganizeImportsSort::Auto },
    ];

    for tt in tests {
        let got = resolve_organize_imports_sort(&tt.preferences);
        assert_eq!(got, tt.want, "{}", tt.name);
    }
}

// utilities_test.go:145
#[test]
fn test_compare_organize_imports_natural_strings() {
    let comparer = get_organize_imports_preset_string_comparer(OrganizeImportsSort::NaturalIgnoreCase);
    struct Test {
        name: &'static str,
        a: &'static str,
        b: &'static str,
        want: i32,
    }
    let tests = [
        Test { name: "numeric runs sort by numeric value", a: "a2", b: "a100", want: -1 },
        Test { name: "numeric runs with equal value use raw tie break", a: "a02", b: "a2", want: -1 },
        Test { name: "accents are folded for primary comparison", a: "À", b: "B", want: -1 },
        Test { name: "raw comparison breaks accent ties", a: "A", b: "À", want: -1 },
        Test { name: "hyphen sorts before slash like Intl.Collator fallback", a: "app-init", b: "app/app", want: -1 },
    ];

    for tt in tests {
        let got = cmp_sign(comparer(tt.a, tt.b));
        assert_eq!(got, tt.want, "{}: comparer({:?}, {:?})", tt.name, tt.a, tt.b);
    }
}

// utilities_test.go:192
fn cmp_sign(value: i32) -> i32 {
    value.signum()
}
