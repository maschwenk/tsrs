use std::fmt;

use super::version::*;

// Go's `\s`: [\t\n\f\r ]
fn is_go_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' ')
}

// [a-z0-9-+.*] with (?i)
fn is_range_operand_char(c: char) -> bool {
    is_letter_fold(c) || is_digit(c) || matches!(c, '-' | '+' | '.' | '*')
}

// https://github.com/npm/node-semver#range-grammar
//
// range-set    ::= range ( logical-or range ) *
// range        ::= hyphen | simple ( ' ' simple ) * | ”
// logical-or   ::= ( ' ' ) * '||' ( ' ' ) *
// \|\|
pub(crate) fn logical_or_reg_exp_split(text: &str) -> Vec<&str> {
    text.split("||").collect()
}

// \s+
pub(crate) fn whitespace_reg_exp_split(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut beg = 0;
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if is_go_space(bytes[i] as char) {
            result.push(&text[beg..i]);
            while i < bytes.len() && is_go_space(bytes[i] as char) {
                i += 1;
            }
            beg = i;
        } else {
            i += 1;
        }
    }
    result.push(&text[beg..]);
    result
}

// https://github.com/npm/node-semver#range-grammar
//
// partial      ::= xr ( '.' xr ( '.' xr qualifier ? )? )?
// xr           ::= 'x' | 'X' | '*' | nr
// nr           ::= '0' | ['1'-'9'] ( ['0'-'9'] ) *
// qualifier    ::= ( '-' pre )? ( '+' build )?
// pre          ::= parts
// build        ::= parts
// parts        ::= part ( '.' part ) *
// part         ::= nr | [-0-9A-Za-z]+
// (?i)^([x*0]|[1-9]\d*)(?:\.([x*0]|[1-9]\d*)(?:\.([x*0]|[1-9]\d*)(?:-([a-z0-9-.]+))?(?:\+([a-z0-9-.]+))?)?)?$
pub(crate) fn partial_reg_exp_find_string_submatch(text: &str) -> Option<[&str; 6]> {
    // [x*0]|[1-9]\d*
    fn component(text: &str) -> Option<usize> {
        match text.as_bytes().first() {
            Some(b'x' | b'X' | b'*' | b'0') => Some(1),
            _ => match_numeric_component(text),
        }
    }
    find_version_like_submatch(text, component)
}

// https://github.com/npm/node-semver#range-grammar
//
// hyphen       ::= partial ' - ' partial
// (?i)^\s*([a-z0-9-+.*]+)\s+-\s+([a-z0-9-+.*]+)\s*$
pub(crate) fn hyphen_reg_exp_find_string_submatch(text: &str) -> Option<[&str; 3]> {
    let mut pos = span(text, is_go_space);
    let left_len = span(&text[pos..], is_range_operand_char);
    if left_len == 0 {
        return None;
    }
    let left = &text[pos..pos + left_len];
    pos += left_len;
    let ws = span(&text[pos..], is_go_space);
    if ws == 0 {
        return None;
    }
    pos += ws;
    if !text[pos..].starts_with('-') {
        return None;
    }
    pos += 1;
    let ws = span(&text[pos..], is_go_space);
    if ws == 0 {
        return None;
    }
    pos += ws;
    let right_len = span(&text[pos..], is_range_operand_char);
    if right_len == 0 {
        return None;
    }
    let right = &text[pos..pos + right_len];
    pos += right_len;
    pos += span(&text[pos..], is_go_space);
    if pos != text.len() {
        return None;
    }
    Some([text, left, right])
}

// https://github.com/npm/node-semver#range-grammar
//
// simple       ::= primitive | partial | tilde | caret
// primitive    ::= ( '<' | '>' | '>=' | '<=' | '=' ) partial
// tilde        ::= '~' partial
// caret        ::= '^' partial
// (?i)^([~^<>=]|<=|>=)?\s*([a-z0-9-+.*]+)$
// The alternatives of the optional first group are tried in leftmost-first order.
pub(crate) fn range_reg_exp_find_string_submatch(text: &str) -> Option<[&str; 3]> {
    fn rest(text: &str) -> Option<&str> {
        let operand = &text[span(text, is_go_space)..];
        if !operand.is_empty() && span(operand, is_range_operand_char) == operand.len() {
            Some(operand)
        } else {
            None
        }
    }
    if matches!(text.as_bytes().first(), Some(b'~' | b'^' | b'<' | b'>' | b'=')) {
        if let Some(operand) = rest(&text[1..]) {
            return Some([text, &text[..1], operand]);
        }
    }
    for op in ["<=", ">="] {
        if let Some(after) = text.strip_prefix(op) {
            if let Some(operand) = rest(after) {
                return Some([text, &text[..2], operand]);
            }
        }
    }
    rest(text).map(|operand| [text, "", operand])
}

#[derive(Clone, Debug, Default)]
pub struct VersionRange {
    pub(crate) alternatives: Vec<Vec<VersionComparator>>,
}

#[derive(Clone, Debug)]
pub(crate) struct VersionComparator {
    pub(crate) operator: ComparatorOperator,
    pub(crate) operand: Version,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub(crate) enum ComparatorOperator {
    RangeLessThan,
    RangeLessThanEqual,
    RangeEqual,
    RangeGreaterThanEqual,
    RangeGreaterThan,
}

impl ComparatorOperator {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ComparatorOperator::RangeLessThan => "<",
            ComparatorOperator::RangeLessThanEqual => "<=",
            ComparatorOperator::RangeEqual => "=",
            ComparatorOperator::RangeGreaterThanEqual => ">=",
            ComparatorOperator::RangeGreaterThan => ">",
        }
    }
}

impl VersionRange {
    pub fn string(&self) -> String {
        let mut sb = String::new();
        format_disjunction(&mut sb, &self.alternatives);
        sb
    }
}

impl fmt::Display for VersionRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string())
    }
}

pub(crate) fn format_disjunction(sb: &mut String, alternatives: &[Vec<VersionComparator>]) {
    let orig_len = sb.len();

    for (i, alternative) in alternatives.iter().enumerate() {
        if i > 0 {
            sb.push_str(" || ");
        }
        format_alternative(sb, alternative);
    }

    if sb.len() == orig_len {
        sb.push('*');
    }
}

pub(crate) fn format_alternative(sb: &mut String, comparators: &[VersionComparator]) {
    for (i, comparator) in comparators.iter().enumerate() {
        if i > 0 {
            sb.push(' ');
        }
        format_comparator(sb, comparator);
    }
}

pub(crate) fn format_comparator(sb: &mut String, comparator: &VersionComparator) {
    sb.push_str(comparator.operator.as_str());
    sb.push_str(&comparator.operand.string());
}

impl VersionRange {
    pub fn test(&self, version: &Version) -> bool {
        test_disjunction(&self.alternatives, version)
    }
}

pub(crate) fn test_disjunction(alternatives: &[Vec<VersionComparator>], version: &Version) -> bool {
    // an empty disjunction is treated as "*" (all versions)
    if alternatives.is_empty() {
        return true;
    }

    for alternative in alternatives {
        if test_alternative(alternative, version) {
            return true;
        }
    }

    false
}

pub(crate) fn test_alternative(alternative: &[VersionComparator], version: &Version) -> bool {
    for comparator in alternative {
        if !test_comparator(comparator, version) {
            return false;
        }
    }
    true
}

pub(crate) fn test_comparator(comparator: &VersionComparator, version: &Version) -> bool {
    let cmp = version.compare(&comparator.operand);
    match comparator.operator {
        ComparatorOperator::RangeLessThan => cmp < 0,
        ComparatorOperator::RangeLessThanEqual => cmp <= 0,
        ComparatorOperator::RangeEqual => cmp == 0,
        ComparatorOperator::RangeGreaterThanEqual => cmp >= 0,
        ComparatorOperator::RangeGreaterThan => cmp > 0,
    }
}

pub fn try_parse_version_range(text: &str) -> Option<VersionRange> {
    let alternatives = parse_alternatives(text)?;
    Some(VersionRange { alternatives })
}

pub(crate) fn parse_alternatives(text: &str) -> Option<Vec<Vec<VersionComparator>>> {
    let mut alternatives = Vec::new();

    let text = text.trim();
    let ranges = logical_or_reg_exp_split(text);
    for r in ranges {
        let r = r.trim();
        if r.is_empty() {
            continue;
        }

        let mut comparators = Vec::new();

        if let Some(hyphen_match) = hyphen_reg_exp_find_string_submatch(r) {
            if let Some(parsed_comparators) = parse_hyphen(hyphen_match[1], hyphen_match[2]) {
                comparators.extend(parsed_comparators);
            } else {
                return None;
            }
        } else {
            for simple in whitespace_reg_exp_split(r) {
                let m = range_reg_exp_find_string_submatch(simple.trim())?;

                if let Some(parsed_comparators) = parse_comparator(m[1], m[2]) {
                    comparators.extend(parsed_comparators);
                } else {
                    return None;
                }
            }
        }

        alternatives.push(comparators);
    }

    Some(alternatives)
}

pub(crate) fn parse_hyphen(left: &str, right: &str) -> Option<Vec<VersionComparator>> {
    let left_result = parse_partial(left)?;

    let right_result = parse_partial(right)?;

    let mut comparators = Vec::new();
    if !is_wildcard(&left_result.major_str) {
        // `MAJOR.*.*-...` gives us `>=MAJOR.0.0 ...`
        comparators.push(VersionComparator {
            operator: ComparatorOperator::RangeGreaterThanEqual,
            operand: left_result.version,
        });
    }

    if !is_wildcard(&right_result.major_str) {
        let operator;
        let mut operand = right_result.version;

        if is_wildcard(&right_result.minor_str) {
            // `...-MAJOR.*.*` gives us `... <(MAJOR+1).0.0`
            operand = operand.increment_major();
            operator = ComparatorOperator::RangeLessThan;
        } else if is_wildcard(&right_result.patch_str) {
            // `...-MAJOR.MINOR.*` gives us `... <MAJOR.(MINOR+1).0`
            operand = operand.increment_minor();
            operator = ComparatorOperator::RangeLessThan;
        } else {
            // `...-MAJOR.MINOR.PATCH` gives us `... <=MAJOR.MINOR.PATCH`
            operator = ComparatorOperator::RangeLessThanEqual;
        }

        comparators.push(VersionComparator { operator, operand });
    }

    Some(comparators)
}

pub(crate) struct PartialVersion {
    pub(crate) version: Version,
    pub(crate) major_str: String,
    pub(crate) minor_str: String,
    pub(crate) patch_str: String,
}

// Produces a "partial" version
pub(crate) fn parse_partial(text: &str) -> Option<PartialVersion> {
    let m = partial_reg_exp_find_string_submatch(text)?;

    let major_str = m[1];
    let mut minor_str = m[2];
    let mut patch_str = m[3];
    let prerelease_str = m[4];
    let build_str = m[5];

    if minor_str.is_empty() {
        minor_str = "*";
    }
    if patch_str.is_empty() {
        patch_str = "*";
    }

    let major_numeric;
    let minor_numeric;
    let patch_numeric;

    if is_wildcard(major_str) {
        major_numeric = 0;
        minor_numeric = 0;
        patch_numeric = 0;
    } else {
        major_numeric = get_uint_component(major_str).ok()?;

        if is_wildcard(minor_str) {
            minor_numeric = 0;
            patch_numeric = 0;
        } else {
            minor_numeric = get_uint_component(minor_str).ok()?;

            if is_wildcard(patch_str) {
                patch_numeric = 0;
            } else {
                patch_numeric = get_uint_component(patch_str).ok()?;
            }
        }
    }

    let mut prerelease = Vec::new();
    if !prerelease_str.is_empty() {
        prerelease = prerelease_str.split('.').map(str::to_string).collect();
    }

    let mut build = Vec::new();
    if !build_str.is_empty() {
        build = build_str.split('.').map(str::to_string).collect();
    }

    let result = PartialVersion {
        version: Version {
            major: major_numeric,
            minor: minor_numeric,
            patch: patch_numeric,
            prerelease,
            build,
        },
        major_str: major_str.to_string(),
        minor_str: minor_str.to_string(),
        patch_str: patch_str.to_string(),
    };

    Some(result)
}

pub(crate) fn parse_comparator(op: &str, text: &str) -> Option<Vec<VersionComparator>> {
    let result = parse_partial(text)?;

    let mut comparators_result = Vec::new();

    if !is_wildcard(&result.major_str) {
        match op {
            "~" => {
                let first = VersionComparator { operator: ComparatorOperator::RangeGreaterThanEqual, operand: result.version.clone() };

                let second_version = if is_wildcard(&result.minor_str) {
                    result.version.increment_major()
                } else {
                    result.version.increment_minor()
                };

                let second = VersionComparator { operator: ComparatorOperator::RangeLessThan, operand: second_version };
                comparators_result = vec![first, second];
            }
            "^" => {
                let first = VersionComparator { operator: ComparatorOperator::RangeGreaterThanEqual, operand: result.version.clone() };

                let second_version = if result.version.major > 0 || is_wildcard(&result.minor_str) {
                    result.version.increment_major()
                } else if result.version.minor > 0 || is_wildcard(&result.patch_str) {
                    result.version.increment_minor()
                } else {
                    result.version.increment_patch()
                };
                let second = VersionComparator { operator: ComparatorOperator::RangeLessThan, operand: second_version };
                comparators_result = vec![first, second];
            }
            "<" | ">=" => {
                let operator = if op == "<" { ComparatorOperator::RangeLessThan } else { ComparatorOperator::RangeGreaterThanEqual };
                let mut version = result.version.clone();
                if is_wildcard(&result.minor_str) || is_wildcard(&result.patch_str) {
                    version.prerelease = vec!["0".to_string()];
                }
                comparators_result = vec![VersionComparator { operator, operand: version }];
            }
            "<=" | ">" => {
                let mut operator = if op == "<=" { ComparatorOperator::RangeLessThanEqual } else { ComparatorOperator::RangeGreaterThan };
                let mut version = result.version.clone();
                if is_wildcard(&result.minor_str) {
                    if operator == ComparatorOperator::RangeLessThanEqual {
                        operator = ComparatorOperator::RangeLessThan;
                    } else {
                        operator = ComparatorOperator::RangeGreaterThanEqual;
                    }

                    version = version.increment_major();
                    version.prerelease = vec!["0".to_string()];
                } else if is_wildcard(&result.patch_str) {
                    if operator == ComparatorOperator::RangeLessThanEqual {
                        operator = ComparatorOperator::RangeLessThan;
                    } else {
                        operator = ComparatorOperator::RangeGreaterThanEqual;
                    }

                    version = version.increment_minor();
                    version.prerelease = vec!["0".to_string()];
                }

                comparators_result = vec![VersionComparator { operator, operand: version }];
            }
            "=" | "" => {
                // normalize empty string to `=`
                let operator = ComparatorOperator::RangeEqual;

                if is_wildcard(&result.minor_str) || is_wildcard(&result.patch_str) {
                    let original_version = &result.version;

                    let mut first_version = original_version.clone();
                    first_version.prerelease = vec!["0".to_string()];

                    let mut second_version = if is_wildcard(&result.minor_str) {
                        original_version.increment_major()
                    } else {
                        original_version.increment_minor()
                    };
                    second_version.prerelease = vec!["0".to_string()];

                    comparators_result = vec![
                        VersionComparator { operator: ComparatorOperator::RangeGreaterThanEqual, operand: first_version },
                        VersionComparator { operator: ComparatorOperator::RangeLessThan, operand: second_version },
                    ];
                } else {
                    comparators_result = vec![VersionComparator { operator, operand: result.version }];
                }
            }
            _ => panic!("Unexpected operator: {op}"),
        }
    } else if op == "<" || op == ">" {
        comparators_result = vec![
            // < 0.0.0-0
            VersionComparator { operator: ComparatorOperator::RangeLessThan, operand: VERSION_ZERO.clone() },
        ];
    }

    Some(comparators_result)
}

pub(crate) fn is_wildcard(text: &str) -> bool {
    text == "*" || text == "x" || text == "X"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcards_have_same_string() {
        let major_wildcard_strings = ["", "*", "*.*", "*.*.*", "x", "x.x", "x.x.x", "X", "X.X", "X.X.X"];

        let minor_wildcard_strings = ["1", "1.*", "1.*.*", "1.x", "1.x.x", "1.X", "1.X.X"];

        let patch_wildcard_strings = ["1.2", "1.2.*", "1.2.x", "1.2.X"];

        let mixed_case_wildcard_strings = ["x", "X", "*", "x.X.x", "X.x.*"];

        assert_all_version_ranges_have_identical_strings("majorWildcardStrings", &major_wildcard_strings);
        assert_all_version_ranges_have_identical_strings("minorWildcardStrings", &minor_wildcard_strings);
        assert_all_version_ranges_have_identical_strings("patchWildcardStrings", &patch_wildcard_strings);
        assert_all_version_ranges_have_identical_strings("mixedCaseWildcardStrings", &mixed_case_wildcard_strings);
    }

    fn assert_all_version_ranges_have_identical_strings(name: &str, strs: &[&str]) {
        for s1 in strs {
            for s2 in strs {
                let v1 = try_parse_version_range(s1).unwrap_or_else(|| panic!("{name}: {s1}"));
                let v2 = try_parse_version_range(s2).unwrap_or_else(|| panic!("{name}: {s2}"));
                assert_eq!(v1.string(), v2.string(), "{name}: {s1} == {s2}");
            }
        }
    }

    struct TestGoodBad<'a> {
        good: &'a [&'a str],
        bad: &'a [&'a str],
    }

    #[test]
    fn test_version_ranges() {
        assert_ranges_good_bad(
            "1",
            TestGoodBad { good: &["1.0.0", "1.9.9", "1.0.0-pre", "1.0.0+build"], bad: &["0.0.0", "2.0.0", "0.0.0-pre", "0.0.0+build"] },
        );
        assert_ranges_good_bad(
            "1.2",
            TestGoodBad { good: &["1.2.0", "1.2.9", "1.2.0-pre", "1.2.0+build"], bad: &["1.1.0", "1.3.0", "1.1.0-pre", "1.1.0+build"] },
        );

        assert_ranges_good_bad(
            "1.2.3",
            TestGoodBad { good: &["1.2.3", "1.2.3+build"], bad: &["1.2.2", "1.2.4", "1.2.2-pre", "1.2.2+build", "1.2.3-pre"] },
        );

        assert_ranges_good_bad(
            "1.2.3-pre",
            TestGoodBad {
                good: &["1.2.3-pre", "1.2.3-pre+build.stuff"],
                bad: &["1.2.3", "1.2.3-pre.0", "1.2.3-pre.9", "1.2.3-pre.0+build", "1.2.3-pre.9+build", "1.2.3+build", "1.2.4"],
            },
        );

        assert_ranges_good_bad("<3.8.0", TestGoodBad { good: &["3.6", "3.7"], bad: &["3.8", "3.9", "4.0"] });

        assert_ranges_good_bad("<=3.8.0", TestGoodBad { good: &["3.6", "3.7", "3.8"], bad: &["3.9", "4.0"] });
        assert_ranges_good_bad(">3.8.0", TestGoodBad { good: &["3.9", "4.0"], bad: &["3.6", "3.7", "3.8"] });
        assert_ranges_good_bad(">=3.8.0", TestGoodBad { good: &["3.8", "3.9", "4.0"], bad: &["3.6", "3.7"] });

        assert_ranges_good_bad("<3.8.0-0", TestGoodBad { good: &["3.6", "3.7"], bad: &["3.8", "3.9", "4.0"] });

        assert_ranges_good_bad("<=3.8.0-0", TestGoodBad { good: &["3.6", "3.7"], bad: &["3.8", "3.9", "4.0"] });

        // Big numbers in prerelease strings.
        let lotsa_ones = "1".repeat(320);
        let range = format!(">=1.2.3-1{lotsa_ones}");
        let good = [format!("1.2.3-1{lotsa_ones}"), format!("1.2.3-11{lotsa_ones}.1"), format!("1.2.3-1{lotsa_ones}.1+build")];
        let bad = [format!("1.2.3-{lotsa_ones}.1+build")];
        let good: Vec<&str> = good.iter().map(String::as_str).collect();
        let bad: Vec<&str> = bad.iter().map(String::as_str).collect();
        assert_ranges_good_bad(&range, TestGoodBad { good: &good, bad: &bad });
    }

    #[test]
    fn test_comparators_of_version_ranges() {
        let comparators_tests: &[(&str, &str, bool)] = &[
            // empty (matches everything)
            ("", "2.0.0", true),
            ("", "2.0.0-0", true),
            ("", "1.1.0", true),
            ("", "1.1.0-0", true),
            ("", "1.0.1", true),
            ("", "1.0.1-0", true),
            ("", "1.0.0", true),
            ("", "1.0.0-0", true),
            ("", "0.0.0", true),
            ("", "0.0.0-0", true),

            // wildcard major (matches everything)
            ("*", "2.0.0", true),
            ("*", "2.0.0-0", true),
            ("*", "1.1.0", true),
            ("*", "1.1.0-0", true),
            ("*", "1.0.1", true),
            ("*", "1.0.1-0", true),
            ("*", "1.0.0", true),
            ("*", "1.0.0-0", true),
            ("*", "0.0.0", true),
            ("*", "0.0.0-0", true),

            // wildcard minor
            ("1", "2.0.0", false),
            ("1", "2.0.0-0", false),
            ("1", "1.1.0", true),
            ("1", "1.1.0-0", true),
            ("1", "1.0.1", true),
            ("1", "1.0.1-0", true),
            ("1", "1.0.0", true),
            ("1", "1.0.0-0", true),
            ("1", "0.0.0", false),
            ("1", "0.0.0-0", false),

            // wildcard patch
            ("1.1", "2.0.0", false),
            ("1.1", "2.0.0-0", false),
            ("1.1", "1.1.0", true),
            ("1.1", "1.1.0-0", true),
            ("1.1", "1.0.1", false),
            ("1.1", "1.0.1-0", false),
            ("1.1", "1.0.0", false),
            ("1.1", "1.0.0-0", false),
            ("1.1", "0.0.0", false),
            ("1.1", "0.0.0-0", false),
            ("1.0", "2.0.0", false),
            ("1.0", "2.0.0-0", false),
            ("1.0", "1.1.0", false),
            ("1.0", "1.1.0-0", false),
            ("1.0", "1.0.1", true),
            ("1.0", "1.0.1-0", true),
            ("1.0", "1.0.0", true),
            ("1.0", "1.0.0-0", true),
            ("1.0", "0.0.0", false),
            ("1.0", "0.0.0-0", false),

            // exact
            ("1.1.0", "2.0.0", false),
            ("1.1.0", "2.0.0-0", false),
            ("1.1.0", "1.1.0", true),
            ("1.1.0", "1.1.0-0", false),
            ("1.1.0", "1.0.1", false),
            ("1.1.0", "1.0.1-0", false),
            ("1.1.0", "1.0.0-0", false),
            ("1.1.0", "1.0.0", false),
            ("1.1.0", "0.0.0", false),
            ("1.1.0", "0.0.0-0", false),
            ("1.1.0-0", "2.0.0", false),
            ("1.1.0-0", "2.0.0-0", false),
            ("1.1.0-0", "1.1.0", false),
            ("1.1.0-0", "1.1.0-0", true),
            ("1.1.0-0", "1.0.1", false),
            ("1.1.0-0", "1.0.1-0", false),
            ("1.1.0-0", "1.0.0-0", false),
            ("1.1.0-0", "1.0.0", false),
            ("1.1.0-0", "0.0.0", false),
            ("1.1.0-0", "0.0.0-0", false),
            ("1.0.1", "2.0.0", false),
            ("1.0.1", "2.0.0-0", false),
            ("1.0.1", "1.1.0", false),
            ("1.0.1", "1.1.0-0", false),
            ("1.0.1", "1.0.1", true),
            ("1.0.1", "1.0.1-0", false),
            ("1.0.1", "1.0.0-0", false),
            ("1.0.1", "1.0.0", false),
            ("1.0.1", "0.0.0", false),
            ("1.0.1", "0.0.0-0", false),
            ("1.0.1-0", "2.0.0", false),
            ("1.0.1-0", "2.0.0-0", false),
            ("1.0.1-0", "1.1.0", false),
            ("1.0.1-0", "1.1.0-0", false),
            ("1.0.1-0", "1.0.1", false),
            ("1.0.1-0", "1.0.1-0", true),
            ("1.0.1-0", "1.0.0-0", false),
            ("1.0.1-0", "1.0.0", false),
            ("1.0.1-0", "0.0.0", false),
            ("1.0.1-0", "0.0.0-0", false),
            ("1.0.0", "2.0.0", false),
            ("1.0.0", "2.0.0-0", false),
            ("1.0.0", "1.1.0", false),
            ("1.0.0", "1.1.0-0", false),
            ("1.0.0", "1.0.1", false),
            ("1.0.0", "1.0.1-0", false),
            ("1.0.0", "1.0.0-0", false),
            ("1.0.0", "1.0.0", true),
            ("1.0.0", "0.0.0", false),
            ("1.0.0", "0.0.0-0", false),
            ("1.0.0-0", "2.0.0", false),
            ("1.0.0-0", "2.0.0-0", false),
            ("1.0.0-0", "1.1.0", false),
            ("1.0.0-0", "1.1.0-0", false),
            ("1.0.0-0", "1.0.1", false),
            ("1.0.0-0", "1.0.1-0", false),
            ("1.0.0-0", "1.0.0", false),
            ("1.0.0-0", "1.0.0-0", true),

            // = wildcard major (matches everything)
            ("=*", "2.0.0", true),
            ("=*", "2.0.0-0", true),
            ("=*", "1.1.0", true),
            ("=*", "1.1.0-0", true),
            ("=*", "1.0.1", true),
            ("=*", "1.0.1-0", true),
            ("=*", "1.0.0", true),
            ("=*", "1.0.0-0", true),
            ("=*", "0.0.0", true),
            ("=*", "0.0.0-0", true),

            // = wildcard minor
            ("=1", "2.0.0", false),
            ("=1", "2.0.0-0", false),
            ("=1", "1.1.0", true),
            ("=1", "1.1.0-0", true),
            ("=1", "1.0.1", true),
            ("=1", "1.0.1-0", true),
            ("=1", "1.0.0", true),
            ("=1", "1.0.0-0", true),
            ("=1", "0.0.0", false),
            ("=1", "0.0.0-0", false),

            // = wildcard patch
            ("=1.1", "2.0.0", false),
            ("=1.1", "2.0.0-0", false),
            ("=1.1", "1.1.0", true),
            ("=1.1", "1.1.0-0", true),
            ("=1.1", "1.0.1", false),
            ("=1.1", "1.0.1-0", false),
            ("=1.1", "1.0.0", false),
            ("=1.1", "1.0.0-0", false),
            ("=1.1", "0.0.0", false),
            ("=1.1", "0.0.0-0", false),
            ("=1.0", "2.0.0", false),
            ("=1.0", "2.0.0-0", false),
            ("=1.0", "1.1.0", false),
            ("=1.0", "1.1.0-0", false),
            ("=1.0", "1.0.1", true),
            ("=1.0", "1.0.1-0", true),
            ("=1.0", "1.0.0", true),
            ("=1.0", "1.0.0-0", true),
            ("=1.0", "0.0.0", false),
            ("=1.0", "0.0.0-0", false),

            // = exact
            ("=1.1.0", "2.0.0", false),
            ("=1.1.0", "2.0.0-0", false),
            ("=1.1.0", "1.1.0", true),
            ("=1.1.0", "1.1.0-0", false),
            ("=1.1.0", "1.0.1", false),
            ("=1.1.0", "1.0.1-0", false),
            ("=1.1.0", "1.0.0-0", false),
            ("=1.1.0", "1.0.0", false),
            ("=1.1.0", "0.0.0", false),
            ("=1.1.0", "0.0.0-0", false),
            ("=1.1.0-0", "2.0.0", false),
            ("=1.1.0-0", "2.0.0-0", false),
            ("=1.1.0-0", "1.1.0", false),
            ("=1.1.0-0", "1.1.0-0", true),
            ("=1.1.0-0", "1.0.1", false),
            ("=1.1.0-0", "1.0.1-0", false),
            ("=1.1.0-0", "1.0.0-0", false),
            ("=1.1.0-0", "1.0.0", false),
            ("=1.1.0-0", "0.0.0", false),
            ("=1.1.0-0", "0.0.0-0", false),
            ("=1.0.1", "2.0.0", false),
            ("=1.0.1", "2.0.0-0", false),
            ("=1.0.1", "1.1.0", false),
            ("=1.0.1", "1.1.0-0", false),
            ("=1.0.1", "1.0.1", true),
            ("=1.0.1", "1.0.1-0", false),
            ("=1.0.1", "1.0.0-0", false),
            ("=1.0.1", "1.0.0", false),
            ("=1.0.1", "0.0.0", false),
            ("=1.0.1", "0.0.0-0", false),
            ("=1.0.1-0", "2.0.0", false),
            ("=1.0.1-0", "2.0.0-0", false),
            ("=1.0.1-0", "1.1.0", false),
            ("=1.0.1-0", "1.1.0-0", false),
            ("=1.0.1-0", "1.0.1", false),
            ("=1.0.1-0", "1.0.1-0", true),
            ("=1.0.1-0", "1.0.0-0", false),
            ("=1.0.1-0", "1.0.0", false),
            ("=1.0.1-0", "0.0.0", false),
            ("=1.0.1-0", "0.0.0-0", false),
            ("=1.0.0", "2.0.0", false),
            ("=1.0.0", "2.0.0-0", false),
            ("=1.0.0", "1.1.0", false),
            ("=1.0.0", "1.1.0-0", false),
            ("=1.0.0", "1.0.1", false),
            ("=1.0.0", "1.0.1-0", false),
            ("=1.0.0", "1.0.0-0", false),
            ("=1.0.0", "1.0.0", true),
            ("=1.0.0", "0.0.0", false),
            ("=1.0.0", "0.0.0-0", false),
            ("=1.0.0-0", "2.0.0", false),
            ("=1.0.0-0", "2.0.0-0", false),
            ("=1.0.0-0", "1.1.0", false),
            ("=1.0.0-0", "1.1.0-0", false),
            ("=1.0.0-0", "1.0.1", false),
            ("=1.0.0-0", "1.0.1-0", false),
            ("=1.0.0-0", "1.0.0", false),
            ("=1.0.0-0", "1.0.0-0", true),

            // > wildcard major (matches nothing)
            (">*", "2.0.0", false),
            (">*", "2.0.0-0", false),
            (">*", "1.1.0", false),
            (">*", "1.1.0-0", false),
            (">*", "1.0.1", false),
            (">*", "1.0.1-0", false),
            (">*", "1.0.0", false),
            (">*", "1.0.0-0", false),
            (">*", "0.0.0", false),
            (">*", "0.0.0-0", false),

            // > wildcard minor
            (">1", "2.0.0", true),
            (">1", "2.0.0-0", true),
            (">1", "1.1.0", false),
            (">1", "1.1.0-0", false),
            (">1", "1.0.1", false),
            (">1", "1.0.1-0", false),
            (">1", "1.0.0", false),
            (">1", "1.0.0-0", false),
            (">1", "0.0.0", false),
            (">1", "0.0.0-0", false),

            // > wildcard patch
            (">1.1", "2.0.0", true),
            (">1.1", "2.0.0-0", true),
            (">1.1", "1.1.0", false),
            (">1.1", "1.1.0-0", false),
            (">1.1", "1.0.1", false),
            (">1.1", "1.0.1-0", false),
            (">1.1", "1.0.0", false),
            (">1.1", "1.0.0-0", false),
            (">1.1", "0.0.0", false),
            (">1.1", "0.0.0-0", false),
            (">1.0", "2.0.0", true),
            (">1.0", "2.0.0-0", true),
            (">1.0", "1.1.0", true),
            (">1.0", "1.1.0-0", true),
            (">1.0", "1.0.1", false),
            (">1.0", "1.0.1-0", false),
            (">1.0", "1.0.0", false),
            (">1.0", "1.0.0-0", false),
            (">1.0", "0.0.0", false),
            (">1.0", "0.0.0-0", false),

            // > exact
            (">1.1.0", "2.0.0", true),
            (">1.1.0", "2.0.0-0", true),
            (">1.1.0", "1.1.0", false),
            (">1.1.0", "1.1.0-0", false),
            (">1.1.0", "1.0.1", false),
            (">1.1.0", "1.0.1-0", false),
            (">1.1.0", "1.0.0", false),
            (">1.1.0", "1.0.0-0", false),
            (">1.1.0", "0.0.0", false),
            (">1.1.0", "0.0.0-0", false),
            (">1.1.0-0", "2.0.0", true),
            (">1.1.0-0", "2.0.0-0", true),
            (">1.1.0-0", "1.1.0", true),
            (">1.1.0-0", "1.1.0-0", false),
            (">1.1.0-0", "1.0.1", false),
            (">1.1.0-0", "1.0.1-0", false),
            (">1.1.0-0", "1.0.0", false),
            (">1.1.0-0", "1.0.0-0", false),
            (">1.1.0-0", "0.0.0", false),
            (">1.1.0-0", "0.0.0-0", false),
            (">1.0.1", "2.0.0", true),
            (">1.0.1", "2.0.0-0", true),
            (">1.0.1", "1.1.0", true),
            (">1.0.1", "1.1.0-0", true),
            (">1.0.1", "1.0.1", false),
            (">1.0.1", "1.0.1-0", false),
            (">1.0.1", "1.0.0", false),
            (">1.0.1", "1.0.0-0", false),
            (">1.0.1", "0.0.0", false),
            (">1.0.1", "0.0.0-0", false),
            (">1.0.1-0", "2.0.0", true),
            (">1.0.1-0", "2.0.0-0", true),
            (">1.0.1-0", "1.1.0", true),
            (">1.0.1-0", "1.1.0-0", true),
            (">1.0.1-0", "1.0.1", true),
            (">1.0.1-0", "1.0.1-0", false),
            (">1.0.1-0", "1.0.0", false),
            (">1.0.1-0", "1.0.0-0", false),
            (">1.0.1-0", "0.0.0", false),
            (">1.0.1-0", "0.0.0-0", false),
            (">1.0.0", "2.0.0", true),
            (">1.0.0", "2.0.0-0", true),
            (">1.0.0", "1.1.0", true),
            (">1.0.0", "1.1.0-0", true),
            (">1.0.0", "1.0.1", true),
            (">1.0.0", "1.0.1-0", true),
            (">1.0.0", "1.0.0", false),
            (">1.0.0", "1.0.0-0", false),
            (">1.0.0", "0.0.0", false),
            (">1.0.0", "0.0.0-0", false),
            (">1.0.0-0", "2.0.0", true),
            (">1.0.0-0", "2.0.0-0", true),
            (">1.0.0-0", "1.1.0", true),
            (">1.0.0-0", "1.1.0-0", true),
            (">1.0.0-0", "1.0.1", true),
            (">1.0.0-0", "1.0.1-0", true),
            (">1.0.0-0", "1.0.0", true),
            (">1.0.0-0", "1.0.0-0", false),
            (">1.0.0-0", "0.0.0", false),
            (">1.0.0-0", "0.0.0-0", false),

            // >= wildcard major (matches everything)
            (">=*", "2.0.0", true),
            (">=*", "2.0.0-0", true),
            (">=*", "1.1.0", true),
            (">=*", "1.1.0-0", true),
            (">=*", "1.0.1", true),
            (">=*", "1.0.1-0", true),
            (">=*", "1.0.0", true),
            (">=*", "1.0.0-0", true),
            (">=*", "0.0.0", true),
            (">=*", "0.0.0-0", true),

            // >= wildcard minor
            (">=1", "2.0.0", true),
            (">=1", "2.0.0-0", true),
            (">=1", "1.1.0", true),
            (">=1", "1.1.0-0", true),
            (">=1", "1.0.1", true),
            (">=1", "1.0.1-0", true),
            (">=1", "1.0.0", true),
            (">=1", "1.0.0-0", true),
            (">=1", "0.0.0", false),
            (">=1", "0.0.0-0", false),

            // >= wildcard patch
            (">=1.1", "2.0.0", true),
            (">=1.1", "2.0.0-0", true),
            (">=1.1", "1.1.0", true),
            (">=1.1", "1.1.0-0", true),
            (">=1.1", "1.0.1", false),
            (">=1.1", "1.0.1-0", false),
            (">=1.1", "1.0.0", false),
            (">=1.1", "1.0.0-0", false),
            (">=1.1", "0.0.0", false),
            (">=1.1", "0.0.0-0", false),
            (">=1.0", "2.0.0", true),
            (">=1.0", "2.0.0-0", true),
            (">=1.0", "1.1.0", true),
            (">=1.0", "1.1.0-0", true),
            (">=1.0", "1.0.1", true),
            (">=1.0", "1.0.1-0", true),
            (">=1.0", "1.0.0", true),
            (">=1.0", "1.0.0-0", true),
            (">=1.0", "0.0.0", false),
            (">=1.0", "0.0.0-0", false),

            // >= exact
            (">=1.1.0", "2.0.0", true),
            (">=1.1.0", "2.0.0-0", true),
            (">=1.1.0", "1.1.0", true),
            (">=1.1.0", "1.1.0-0", false),
            (">=1.1.0", "1.0.1", false),
            (">=1.1.0", "1.0.1-0", false),
            (">=1.1.0", "1.0.0", false),
            (">=1.1.0", "1.0.0-0", false),
            (">=1.1.0", "0.0.0", false),
            (">=1.1.0", "0.0.0-0", false),
            (">=1.1.0-0", "2.0.0", true),
            (">=1.1.0-0", "2.0.0-0", true),
            (">=1.1.0-0", "1.1.0", true),
            (">=1.1.0-0", "1.1.0-0", true),
            (">=1.1.0-0", "1.0.1", false),
            (">=1.1.0-0", "1.0.1-0", false),
            (">=1.1.0-0", "1.0.0", false),
            (">=1.1.0-0", "1.0.0-0", false),
            (">=1.1.0-0", "0.0.0", false),
            (">=1.1.0-0", "0.0.0-0", false),
            (">=1.0.1", "2.0.0", true),
            (">=1.0.1", "2.0.0-0", true),
            (">=1.0.1", "1.1.0", true),
            (">=1.0.1", "1.1.0-0", true),
            (">=1.0.1", "1.0.1", true),
            (">=1.0.1", "1.0.1-0", false),
            (">=1.0.1", "1.0.0", false),
            (">=1.0.1", "1.0.0-0", false),
            (">=1.0.1", "0.0.0", false),
            (">=1.0.1", "0.0.0-0", false),
            (">=1.0.1-0", "2.0.0", true),
            (">=1.0.1-0", "2.0.0-0", true),
            (">=1.0.1-0", "1.1.0", true),
            (">=1.0.1-0", "1.1.0-0", true),
            (">=1.0.1-0", "1.0.1", true),
            (">=1.0.1-0", "1.0.1-0", true),
            (">=1.0.1-0", "1.0.0", false),
            (">=1.0.1-0", "1.0.0-0", false),
            (">=1.0.1-0", "0.0.0", false),
            (">=1.0.1-0", "0.0.0-0", false),
            (">=1.0.0", "2.0.0", true),
            (">=1.0.0", "2.0.0-0", true),
            (">=1.0.0", "1.1.0", true),
            (">=1.0.0", "1.1.0-0", true),
            (">=1.0.0", "1.0.1", true),
            (">=1.0.0", "1.0.1-0", true),
            (">=1.0.0", "1.0.0", true),
            (">=1.0.0", "1.0.0-0", false),
            (">=1.0.0", "0.0.0", false),
            (">=1.0.0", "0.0.0-0", false),
            (">=1.0.0-0", "2.0.0", true),
            (">=1.0.0-0", "2.0.0-0", true),
            (">=1.0.0-0", "1.1.0", true),
            (">=1.0.0-0", "1.1.0-0", true),
            (">=1.0.0-0", "1.0.1", true),
            (">=1.0.0-0", "1.0.1-0", true),
            (">=1.0.0-0", "1.0.0", true),
            (">=1.0.0-0", "1.0.0-0", true),
            (">=1.0.0-0", "0.0.0", false),
            (">=1.0.0-0", "0.0.0-0", false),

            // < wildcard major (matches nothing)
            ("<*", "2.0.0", false),
            ("<*", "2.0.0-0", false),
            ("<*", "1.1.0", false),
            ("<*", "1.1.0-0", false),
            ("<*", "1.0.1", false),
            ("<*", "1.0.1-0", false),
            ("<*", "1.0.0", false),
            ("<*", "1.0.0-0", false),
            ("<*", "0.0.0", false),
            ("<*", "0.0.0-0", false),

            // < wildcard minor
            ("<1", "2.0.0", false),
            ("<1", "2.0.0-0", false),
            ("<1", "1.1.0", false),
            ("<1", "1.1.0-0", false),
            ("<1", "1.0.1", false),
            ("<1", "1.0.1-0", false),
            ("<1", "1.0.0", false),
            ("<1", "1.0.0-0", false),
            ("<1", "0.0.0", true),
            ("<1", "0.0.0-0", true),

            // < wildcard patch
            ("<1.1", "2.0.0", false),
            ("<1.1", "2.0.0-0", false),
            ("<1.1", "1.1.0", false),
            ("<1.1", "1.1.0-0", false),
            ("<1.1", "1.0.1", true),
            ("<1.1", "1.0.1-0", true),
            ("<1.1", "1.0.0", true),
            ("<1.1", "1.0.0-0", true),
            ("<1.1", "0.0.0", true),
            ("<1.1", "0.0.0-0", true),
            ("<1.0", "2.0.0", false),
            ("<1.0", "2.0.0-0", false),
            ("<1.0", "1.1.0", false),
            ("<1.0", "1.1.0-0", false),
            ("<1.0", "1.0.1", false),
            ("<1.0", "1.0.1-0", false),
            ("<1.0", "1.0.0", false),
            ("<1.0", "1.0.0-0", false),
            ("<1.0", "0.0.0", true),
            ("<1.0", "0.0.0-0", true),

            // < exact
            ("<1.1.0", "2.0.0", false),
            ("<1.1.0", "2.0.0-0", false),
            ("<1.1.0", "1.1.0", false),
            ("<1.1.0", "1.1.0-0", true),
            ("<1.1.0", "1.0.1", true),
            ("<1.1.0", "1.0.1-0", true),
            ("<1.1.0", "1.0.0", true),
            ("<1.1.0", "1.0.0-0", true),
            ("<1.1.0", "0.0.0", true),
            ("<1.1.0", "0.0.0-0", true),
            ("<1.1.0-0", "2.0.0", false),
            ("<1.1.0-0", "2.0.0-0", false),
            ("<1.1.0-0", "1.1.0", false),
            ("<1.1.0-0", "1.1.0-0", false),
            ("<1.1.0-0", "1.0.1", true),
            ("<1.1.0-0", "1.0.1-0", true),
            ("<1.1.0-0", "1.0.0", true),
            ("<1.1.0-0", "1.0.0-0", true),
            ("<1.1.0-0", "0.0.0", true),
            ("<1.1.0-0", "0.0.0-0", true),
            ("<1.0.1", "2.0.0", false),
            ("<1.0.1", "2.0.0-0", false),
            ("<1.0.1", "1.1.0", false),
            ("<1.0.1", "1.1.0-0", false),
            ("<1.0.1", "1.0.1", false),
            ("<1.0.1", "1.0.1-0", true),
            ("<1.0.1", "1.0.0", true),
            ("<1.0.1", "1.0.0-0", true),
            ("<1.0.1", "0.0.0", true),
            ("<1.0.1", "0.0.0-0", true),
            ("<1.0.1-0", "2.0.0", false),
            ("<1.0.1-0", "2.0.0-0", false),
            ("<1.0.1-0", "1.1.0", false),
            ("<1.0.1-0", "1.1.0-0", false),
            ("<1.0.1-0", "1.0.1", false),
            ("<1.0.1-0", "1.0.1-0", false),
            ("<1.0.1-0", "1.0.0", true),
            ("<1.0.1-0", "1.0.0-0", true),
            ("<1.0.1-0", "0.0.0", true),
            ("<1.0.1-0", "0.0.0-0", true),
            ("<1.0.0", "2.0.0", false),
            ("<1.0.0", "2.0.0-0", false),
            ("<1.0.0", "1.1.0", false),
            ("<1.0.0", "1.1.0-0", false),
            ("<1.0.0", "1.0.1", false),
            ("<1.0.0", "1.0.1-0", false),
            ("<1.0.0", "1.0.0", false),
            ("<1.0.0", "1.0.0-0", true),
            ("<1.0.0", "0.0.0", true),
            ("<1.0.0", "0.0.0-0", true),
            ("<1.0.0-0", "2.0.0", false),
            ("<1.0.0-0", "2.0.0-0", false),
            ("<1.0.0-0", "1.1.0", false),
            ("<1.0.0-0", "1.1.0-0", false),
            ("<1.0.0-0", "1.0.1", false),
            ("<1.0.0-0", "1.0.1-0", false),
            ("<1.0.0-0", "1.0.0", false),
            ("<1.0.0-0", "1.0.0-0", false),
            ("<1.0.0-0", "0.0.0", true),
            ("<1.0.0-0", "0.0.0-0", true),

            // <= wildcard major (matches everything)
            ("<=*", "2.0.0", true),
            ("<=*", "2.0.0-0", true),
            ("<=*", "1.1.0", true),
            ("<=*", "1.1.0-0", true),
            ("<=*", "1.0.1", true),
            ("<=*", "1.0.1-0", true),
            ("<=*", "1.0.0", true),
            ("<=*", "1.0.0-0", true),
            ("<=*", "0.0.0", true),
            ("<=*", "0.0.0-0", true),

            // <= wildcard minor
            ("<=1", "2.0.0", false),
            ("<=1", "2.0.0-0", false),
            ("<=1", "1.1.0", true),
            ("<=1", "1.1.0-0", true),
            ("<=1", "1.0.1", true),
            ("<=1", "1.0.1-0", true),
            ("<=1", "1.0.0", true),
            ("<=1", "1.0.0-0", true),
            ("<=1", "0.0.0", true),
            ("<=1", "0.0.0-0", true),

            // <= wildcard patch
            ("<=1.1", "2.0.0", false),
            ("<=1.1", "2.0.0-0", false),
            ("<=1.1", "1.1.0", true),
            ("<=1.1", "1.1.0-0", true),
            ("<=1.1", "1.0.1", true),
            ("<=1.1", "1.0.1-0", true),
            ("<=1.1", "1.0.0", true),
            ("<=1.1", "1.0.0-0", true),
            ("<=1.1", "0.0.0", true),
            ("<=1.1", "0.0.0-0", true),
            ("<=1.0", "2.0.0", false),
            ("<=1.0", "2.0.0-0", false),
            ("<=1.0", "1.1.0", false),
            ("<=1.0", "1.1.0-0", false),
            ("<=1.0", "1.0.1", true),
            ("<=1.0", "1.0.1-0", true),
            ("<=1.0", "1.0.0", true),
            ("<=1.0", "1.0.0-0", true),
            ("<=1.0", "0.0.0", true),
            ("<=1.0", "0.0.0-0", true),

            // <= exact
            ("<=1.1.0", "2.0.0", false),
            ("<=1.1.0", "2.0.0-0", false),
            ("<=1.1.0", "1.1.0", true),
            ("<=1.1.0", "1.1.0-0", true),
            ("<=1.1.0", "1.0.1", true),
            ("<=1.1.0", "1.0.1-0", true),
            ("<=1.1.0", "1.0.0", true),
            ("<=1.1.0", "1.0.0-0", true),
            ("<=1.1.0", "0.0.0", true),
            ("<=1.1.0", "0.0.0-0", true),
            ("<=1.1.0-0", "2.0.0", false),
            ("<=1.1.0-0", "2.0.0-0", false),
            ("<=1.1.0-0", "1.1.0", false),
            ("<=1.1.0-0", "1.1.0-0", true),
            ("<=1.1.0-0", "1.0.1", true),
            ("<=1.1.0-0", "1.0.1-0", true),
            ("<=1.1.0-0", "1.0.0", true),
            ("<=1.1.0-0", "1.0.0-0", true),
            ("<=1.1.0-0", "0.0.0", true),
            ("<=1.1.0-0", "0.0.0-0", true),
            ("<=1.0.1", "2.0.0", false),
            ("<=1.0.1", "2.0.0-0", false),
            ("<=1.0.1", "1.1.0", false),
            ("<=1.0.1", "1.1.0-0", false),
            ("<=1.0.1", "1.0.1", true),
            ("<=1.0.1", "1.0.1-0", true),
            ("<=1.0.1", "1.0.0", true),
            ("<=1.0.1", "1.0.0-0", true),
            ("<=1.0.1", "0.0.0", true),
            ("<=1.0.1", "0.0.0-0", true),
            ("<=1.0.1-0", "2.0.0", false),
            ("<=1.0.1-0", "2.0.0-0", false),
            ("<=1.0.1-0", "1.1.0", false),
            ("<=1.0.1-0", "1.1.0-0", false),
            ("<=1.0.1-0", "1.0.1", false),
            ("<=1.0.1-0", "1.0.1-0", true),
            ("<=1.0.1-0", "1.0.0", true),
            ("<=1.0.1-0", "1.0.0-0", true),
            ("<=1.0.1-0", "0.0.0", true),
            ("<=1.0.1-0", "0.0.0-0", true),
            ("<=1.0.0", "2.0.0", false),
            ("<=1.0.0", "2.0.0-0", false),
            ("<=1.0.0", "1.1.0", false),
            ("<=1.0.0", "1.1.0-0", false),
            ("<=1.0.0", "1.0.1", false),
            ("<=1.0.0", "1.0.1-0", false),
            ("<=1.0.0", "1.0.0", true),
            ("<=1.0.0", "1.0.0-0", true),
            ("<=1.0.0", "0.0.0", true),
            ("<=1.0.0", "0.0.0-0", true),
            ("<=1.0.0-0", "2.0.0", false),
            ("<=1.0.0-0", "2.0.0-0", false),
            ("<=1.0.0-0", "1.1.0", false),
            ("<=1.0.0-0", "1.1.0-0", false),
            ("<=1.0.0-0", "1.0.1", false),
            ("<=1.0.0-0", "1.0.1-0", false),
            ("<=1.0.0-0", "1.0.0", false),
            ("<=1.0.0-0", "1.0.0-0", true),
            ("<=1.0.0-0", "0.0.0", true),
            ("<=1.0.0-0", "0.0.0-0", true),

            // https://github.com/microsoft/TypeScript/issues/50909
            (">4.8", "4.9.0-beta", true),
            (">=4.9", "4.9.0-beta", true),
            ("<4.9", "4.9.0-beta", false),
            ("<=4.8", "4.9.0-beta", false),
        ];
        for &(range_text, version_text, expected) in comparators_tests {
            assert_range_test("comparators", range_text, version_text, expected);
        }
    }

    #[test]
    fn test_conjunctions_of_version_ranges() {
        let conjunction_tests: &[(&str, &str, bool)] = &[
            (">1.0.0 <2.0.0", "1.0.1", true),
            (">1.0.0 <2.0.0", "2.0.0", false),
            (">1.0.0 <2.0.0", "1.0.0", false),
            (">1 >2", "3.0.0", true),
        ];
        for &(range_text, version_text, expected) in conjunction_tests {
            assert_range_test("conjunctions", range_text, version_text, expected);
        }
    }

    #[test]
    fn test_disjunctions_of_version_ranges() {
        let disjunction_tests: &[(&str, &str, bool)] = &[
            (">1.0.0 || <1.0.0", "1.0.1", true),
            (">1.0.0 || <1.0.0", "0.0.1", true),
            (">1.0.0 || <1.0.0", "1.0.0", false),
            (">1.0.0 || <1.0.0", "0.0.0", true),
            (">=1.0.0 <2.0.0 || >=3.0.0 <4.0.0", "1.0.0", true),
            (">=1.0.0 <2.0.0 || >=3.0.0 <4.0.0", "2.0.0", false),
            (">=1.0.0 <2.0.0 || >=3.0.0 <4.0.0", "3.0.0", true),
        ];
        for &(range_text, version_text, expected) in disjunction_tests {
            assert_range_test("disjunctions", range_text, version_text, expected);
        }
    }

    #[test]
    fn test_hyphens_of_version_ranges() {
        let hyphen_tests: &[(&str, &str, bool)] = &[
            ("1.0.0 - 2.0.0", "1.0.0", true),
            ("1.0.0 - 2.0.0", "1.0.1", true),
            ("1.0.0 - 2.0.0", "2.0.0", true),
            ("1.0.0 - 2.0.0", "2.0.1", false),
            ("1.0.0 - 2.0.0", "0.9.9", false),
            ("1.0.0 - 2.0.0", "3.0.0", false),
        ];
        for &(range_text, version_text, expected) in hyphen_tests {
            assert_range_test("hyphens", range_text, version_text, expected);
        }
    }

    #[test]
    fn test_tildes_of_version_ranges() {
        let tilde_tests: &[(&str, &str, bool)] = &[
            ("~0", "0.0.0", true),
            ("~0", "0.1.0", true),
            ("~0", "0.1.2", true),
            ("~0", "0.1.9", true),
            ("~0", "1.0.0", false),
            ("~0.1", "0.1.0", true),
            ("~0.1", "0.1.2", true),
            ("~0.1", "0.1.9", true),
            ("~0.1", "0.2.0", false),
            ("~0.1.2", "0.1.2", true),
            ("~0.1.2", "0.1.9", true),
            ("~0.1.2", "0.2.0", false),
            ("~1.0.0", "1.0.0", true),
            ("~1.0.0", "1.0.1", true),
            ("~1", "1.0.0", true),
            ("~1", "1.2.0", true),
            ("~1", "1.2.3", true),
            ("~1", "0.0.0", false),
            ("~1", "2.0.0", false),
            ("~1.2", "1.2.0", true),
            ("~1.2", "1.2.3", true),
            ("~1.2", "1.1.0", false),
            ("~1.2", "1.3.0", false),
            ("~1.2.3", "1.2.3", true),
            ("~1.2.3", "1.2.9", true),
            ("~1.2.3", "1.1.0", false),
            ("~1.2.3", "1.3.0", false),
        ];
        for &(range_text, version_text, expected) in tilde_tests {
            assert_range_test("tilde", range_text, version_text, expected);
        }
    }

    #[test]
    fn test_carets_of_version_ranges() {
        let caret_tests: &[(&str, &str, bool)] = &[
            ("^0", "0.0.0", true),
            ("^0", "0.1.0", true),
            ("^0", "0.9.0", true),
            ("^0", "0.1.2", true),
            ("^0", "0.1.9", true),
            ("^0", "1.0.0", false),
            ("^0.1", "0.1.0", true),
            ("^0.1", "0.1.2", true),
            ("^0.1", "0.1.9", true),
            ("^0.1.2", "0.1.2", true),
            ("^0.1.2", "0.1.9", true),
            ("^0.1.2", "0.0.0", false),
            ("^0.1.2", "0.2.0", false),
            ("^0.1.2", "1.0.0", false),
            ("^1", "1.0.0", true),
            ("^1", "1.2.0", true),
            ("^1", "1.2.3", true),
            ("^1", "1.9.0", true),
            ("^1", "0.0.0", false),
            ("^1", "2.0.0", false),
            ("^1.2", "1.2.0", true),
            ("^1.2", "1.2.3", true),
            ("^1.2", "1.9.0", true),
            ("^1.2", "1.1.0", false),
            ("^1.2", "2.0.0", false),
            ("^1.2.3", "1.2.3", true),
            ("^1.2.3", "1.9.0", true),
            ("^1.2.3", "1.2.2", false),
            ("^1.2.3", "2.0.0", false),
        ];
        for &(range_text, version_text, expected) in caret_tests {
            assert_range_test("caret", range_text, version_text, expected);
        }
    }

    fn assert_ranges_good_bad(version_range_string: &str, tests: TestGoodBad) {
        let version_range = try_parse_version_range(version_range_string).unwrap_or_else(|| panic!("{version_range_string}"));
        for good in tests.good {
            let v = try_parse_version(good).unwrap();
            assert!(version_range.test(&v), "{good} should be matched by range {version_range_string}");
        }

        for bad in tests.bad {
            let v = try_parse_version(bad).unwrap();
            assert!(!version_range.test(&v), "{bad} should not be matched by range {version_range_string}");
        }
    }

    fn assert_range_test(name: &str, range_text: &str, version_text: &str, in_range: bool) {
        let test_name = format!("{name} (version {version_text} in range {range_text}) == {in_range}");
        let version_range = try_parse_version_range(range_text).unwrap_or_else(|| panic!("{test_name}"));
        let version = try_parse_version(version_text).unwrap_or_else(|e| panic!("{test_name}: {e}"));
        assert_eq!(version_range.test(&version), in_range, "{test_name}");
    }
}
