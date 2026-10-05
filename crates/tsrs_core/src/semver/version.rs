use std::cmp::Ordering;
use std::fmt;
use std::sync::LazyLock;

// Go's `(?i)[a-z]` also matches U+017F (folds to 's') and U+212A (folds to 'k').
pub(crate) fn is_letter_fold(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '\u{17F}' || c == '\u{212A}'
}

pub(crate) fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

// [a-z0-9-.] with (?i)
pub(crate) fn is_identifier_or_dot_char(c: char) -> bool {
    is_letter_fold(c) || is_digit(c) || c == '-' || c == '.'
}

// Length in bytes of the longest prefix of `text` whose chars all satisfy `f`.
pub(crate) fn span(text: &str, f: impl Fn(char) -> bool) -> usize {
    text.char_indices().find(|&(_, c)| !f(c)).map_or(text.len(), |(i, _)| i)
}

// 0|[1-9]\d*
pub(crate) fn match_numeric_component(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    match b.first() {
        Some(b'0') => Some(1),
        Some(b'1'..=b'9') => Some(1 + span(&text[1..], is_digit)),
        _ => None,
    }
}

// Shared shape of versionRegexp and partialRegExp:
// ^(C)(?:\.(C)(?:\.(C)(?:-([a-z0-9-.]+))?(?:\+([a-z0-9-.]+))?)?)?$
// Every group is deterministic and anything a skipped optional group would have consumed
// must then be matched by `$`, so a greedy parse followed by an end check is equivalent.
// Non-participating groups are "", as in Go's FindStringSubmatch.
pub(crate) fn find_version_like_submatch(text: &str, component: fn(&str) -> Option<usize>) -> Option<[&str; 6]> {
    let mut m: [&str; 6] = [""; 6];
    let n = component(text)?;
    m[1] = &text[..n];
    let mut pos = n;
    if let Some(minor_len) = text[pos..].strip_prefix('.').and_then(component) {
        m[2] = &text[pos + 1..pos + 1 + minor_len];
        pos += 1 + minor_len;
        if let Some(patch_len) = text[pos..].strip_prefix('.').and_then(component) {
            m[3] = &text[pos + 1..pos + 1 + patch_len];
            pos += 1 + patch_len;
            if let Some(rest) = text[pos..].strip_prefix('-') {
                let l = span(rest, is_identifier_or_dot_char);
                if l > 0 {
                    m[4] = &rest[..l];
                    pos += 1 + l;
                }
            }
            if let Some(rest) = text[pos..].strip_prefix('+') {
                let l = span(rest, is_identifier_or_dot_char);
                if l > 0 {
                    m[5] = &rest[..l];
                    pos += 1 + l;
                }
            }
        }
    }
    if pos != text.len() {
        return None;
    }
    m[0] = text;
    Some(m)
}

// https://semver.org/#spec-item-2
// > A normal version number MUST take the form X.Y.Z where X, Y, and Z are non-negative
// > integers, and MUST NOT contain leading zeroes. X is the major version, Y is the minor
// > version, and Z is the patch version. Each element MUST increase numerically.
//
// NOTE: We differ here in that we allow X and X.Y, with missing parts having the default
// value of `0`.
// (?i)^(0|[1-9]\d*)(?:\.(0|[1-9]\d*)(?:\.(0|[1-9]\d*)(?:-([a-z0-9-.]+))?(?:\+([a-z0-9-.]+))?)?)?$
pub(crate) fn version_regexp_find_string_submatch(text: &str) -> Option<[&str; 6]> {
    find_version_like_submatch(text, match_numeric_component)
}

// (?i)^(?:0|[1-9]\d*|[a-z-][a-z0-9-]*)$
fn is_prerelease_part(part: &str) -> bool {
    if part == "0" {
        return true;
    }
    let mut chars = part.chars();
    match chars.next() {
        Some('1'..='9') => chars.all(is_digit),
        Some(c) if is_letter_fold(c) || c == '-' => chars.all(|c| is_letter_fold(c) || is_digit(c) || c == '-'),
        _ => false,
    }
}

// https://semver.org/#spec-item-9
// > A pre-release version MAY be denoted by appending a hyphen and a series of dot separated
// > identifiers immediately following the patch version. Identifiers MUST comprise only ASCII
// > alphanumerics and hyphen [0-9A-Za-z-]. Identifiers MUST NOT be empty. Numeric identifiers
// > MUST NOT include leading zeroes.
// (?i)^(?:0|[1-9]\d*|[a-z-][a-z0-9-]*)(?:\.(?:0|[1-9]\d*|[a-zA-Z-][a-zA-Z0-9-]*))*$
pub(crate) fn prerelease_regexp_match_string(text: &str) -> bool {
    text.split('.').all(is_prerelease_part)
}

// (?i)^[a-z0-9-]+$
fn is_build_part(part: &str) -> bool {
    !part.is_empty() && part.chars().all(|c| is_letter_fold(c) || is_digit(c) || c == '-')
}

// https://semver.org/#spec-item-10
// > Build metadata MAY be denoted by appending a plus sign and a series of dot separated
// > identifiers immediately following the patch or pre-release version. Identifiers MUST
// > comprise only ASCII alphanumerics and hyphen [0-9A-Za-z-]. Identifiers MUST NOT be empty.
// (?i)^[a-z0-9-]+(?:\.[a-z0-9-]+)*$
pub(crate) fn build_reg_exp_match_string(text: &str) -> bool {
    text.split('.').all(is_build_part)
}

// https://semver.org/#spec-item-9
// > Numeric identifiers MUST NOT include leading zeroes.
// ^(?:0|[1-9]\d*)$
pub(crate) fn numeric_identifier_reg_exp_match_string(text: &str) -> bool {
    match_numeric_component(text) == Some(text.len())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Version {
    pub(crate) major: u32,
    pub(crate) minor: u32,
    pub(crate) patch: u32,
    pub(crate) prerelease: Vec<String>,
    pub(crate) build: Vec<String>,
}

pub(crate) static VERSION_ZERO: LazyLock<Version> = LazyLock::new(|| Version {
    prerelease: vec!["0".to_string()],
    ..Default::default()
});

impl Version {
    pub(crate) fn increment_major(&self) -> Version {
        Version { major: self.major + 1, ..Default::default() }
    }

    pub(crate) fn increment_minor(&self) -> Version {
        Version { major: self.major, minor: self.minor + 1, ..Default::default() }
    }

    pub(crate) fn increment_patch(&self) -> Version {
        Version { major: self.major, minor: self.minor, patch: self.patch + 1, ..Default::default() }
    }
}

pub(crate) const COMPARISON_LESS_THAN: i32 = -1;
pub(crate) const COMPARISON_EQUAL_TO: i32 = 0;
pub(crate) const COMPARISON_GREATER_THAN: i32 = 1;

fn cmp_compare<T: Ord + Copy>(a: T, b: T) -> i32 {
    match a.cmp(&b) {
        Ordering::Less => COMPARISON_LESS_THAN,
        Ordering::Equal => COMPARISON_EQUAL_TO,
        Ordering::Greater => COMPARISON_GREATER_THAN,
    }
}

impl Version {
    pub fn compare(&self, b: &Version) -> i32 {
        // https://semver.org/#spec-item-11
        // > Precedence is determined by the first difference when comparing each of these
        // > identifiers from left to right as follows: Major, minor, and patch versions are
        // > always compared numerically.
        //
        // https://semver.org/#spec-item-11
        // > Precedence for two pre-release versions with the same major, minor, and patch version
        // > MUST be determined by comparing each dot separated identifier from left to right until
        // > a difference is found [...]
        //
        // https://semver.org/#spec-item-11
        // > Build metadata does not figure into precedence
        let a = self;
        if std::ptr::eq(a, b) {
            return COMPARISON_EQUAL_TO;
        }

        let r = cmp_compare(a.major, b.major);
        if r != 0 {
            return r;
        }

        let r = cmp_compare(a.minor, b.minor);
        if r != 0 {
            return r;
        }

        let r = cmp_compare(a.patch, b.patch);
        if r != 0 {
            return r;
        }

        compare_pre_release_identifiers(&a.prerelease, &b.prerelease)
    }
}

pub(crate) fn compare_pre_release_identifiers(left: &[String], right: &[String]) -> i32 {
    // https://semver.org/#spec-item-11
    // > When major, minor, and patch are equal, a pre-release version has lower precedence
    // > than a normal version.
    if left.is_empty() {
        if right.is_empty() {
            return COMPARISON_EQUAL_TO;
        }
        return COMPARISON_GREATER_THAN;
    } else if right.is_empty() {
        return COMPARISON_LESS_THAN;
    }

    // https://semver.org/#spec-item-11
    // > Precedence for two pre-release versions with the same major, minor, and patch version
    // > MUST be determined by comparing each dot separated identifier from left to right until
    // > a difference is found [...]
    for (l, r) in left.iter().zip(right.iter()) {
        let c = compare_pre_release_identifier(l, r);
        if c != 0 {
            return c;
        }
    }
    cmp_compare(left.len(), right.len())
}

pub(crate) fn compare_pre_release_identifier(left: &str, right: &str) -> i32 {
    // https://semver.org/#spec-item-11
    // > Precedence for two pre-release versions with the same major, minor, and patch version
    // > MUST be determined by comparing each dot separated identifier from left to right until
    // > a difference is found [...]
    let compare_result = cmp_compare(left, right);
    if compare_result == 0 {
        return compare_result;
    }

    let left_is_numeric = numeric_identifier_reg_exp_match_string(left);
    let right_is_numeric = numeric_identifier_reg_exp_match_string(right);

    if left_is_numeric || right_is_numeric {
        // https://semver.org/#spec-item-11
        // > Numeric identifiers always have lower precedence than non-numeric identifiers.
        if !right_is_numeric {
            return COMPARISON_LESS_THAN;
        }
        if !left_is_numeric {
            return COMPARISON_GREATER_THAN;
        }

        // https://semver.org/#spec-item-11
        // > identifiers consisting of only digits are compared numerically
        let left_as_number = get_uint_component(left);
        let right_as_number = get_uint_component(right);
        match (left_as_number, right_as_number) {
            (Ok(l), Ok(r)) => return cmp_compare(l, r),
            _ => {
                // This should only happen in the event of an overflow.
                // If so, use the lengths or fall back to string comparison.
                let left_len = left.len();
                let right_len = right.len();
                let len_compare = cmp_compare(left_len, right_len);
                if len_compare == 0 {
                    return compare_result;
                } else {
                    return len_compare;
                }
            }
        }
    }

    // https://semver.org/#spec-item-11
    // > identifiers with letters or hyphens are compared lexically in ASCII sort order.
    compare_result
}

impl Version {
    pub fn string(&self) -> String {
        let mut sb = format!("{}.{}.{}", self.major, self.minor, self.patch);
        if !self.prerelease.is_empty() {
            sb.push('-');
            sb.push_str(&self.prerelease.join("."));
        }
        if !self.build.is_empty() {
            sb.push('+');
            sb.push_str(&self.build.join("."));
        }
        sb
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string())
    }
}

fn semver_parse_error(orig_input: &str) -> String {
    format!("Could not parse version string from {orig_input:?}")
}

pub fn try_parse_version(text: &str) -> Result<Version, String> {
    let mut result = Version::default();

    let Some(m) = version_regexp_find_string_submatch(text) else {
        return Err(semver_parse_error(text));
    };

    let major_str = m[1];
    let minor_str = m[2];
    let patch_str = m[3];
    let prerelease_str = m[4];
    let build_str = m[5];

    result.major = get_uint_component(major_str)?;

    if !minor_str.is_empty() {
        result.minor = get_uint_component(minor_str)?;
    }

    if !patch_str.is_empty() {
        result.patch = get_uint_component(patch_str)?;
    }

    if !prerelease_str.is_empty() {
        if !prerelease_regexp_match_string(prerelease_str) {
            return Err(semver_parse_error(text));
        }

        result.prerelease = prerelease_str.split('.').map(str::to_string).collect();
    }
    if !build_str.is_empty() {
        if !build_reg_exp_match_string(build_str) {
            return Err(semver_parse_error(text));
        }

        result.build = build_str.split('.').map(str::to_string).collect();
    }

    Ok(result)
}

pub fn must_parse(text: &str) -> Version {
    match try_parse_version(text) {
        Ok(v) => v,
        Err(err) => panic!("{err}"),
    }
}

// strconv.ParseUint(text, 10, 32)
pub(crate) fn get_uint_component(text: &str) -> Result<u32, String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("strconv.ParseUint: parsing {text:?}: invalid syntax"));
    }
    let mut r: u32 = 0;
    for b in text.bytes() {
        match r.checked_mul(10).and_then(|v| v.checked_add((b - b'0') as u32)) {
            Some(v) => r = v,
            None => return Err(format!("strconv.ParseUint: parsing {text:?}: value out of range")),
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u32, minor: u32, patch: u32, prerelease: &[&str], build: &[&str]) -> Version {
        Version {
            major,
            minor,
            patch,
            prerelease: prerelease.iter().map(|s| s.to_string()).collect(),
            build: build.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn test_try_parse_semver() {
        let tests = [
            ("1.2.3-pre.4+build.5", v(1, 2, 3, &["pre", "4"], &["build", "5"])),
            ("1.2.3-pre.4", v(1, 2, 3, &["pre", "4"], &[])),
            ("1.2.3+build.4", v(1, 2, 3, &[], &["build", "4"])),
            ("1.2.3", v(1, 2, 3, &[], &[])),
        ];

        for (input, out) in tests {
            let got = try_parse_version(input).unwrap();
            assert_version(&got, &out);
        }
    }

    #[test]
    fn test_version_string() {
        let tests = [
            (v(1, 2, 3, &["pre", "4"], &["build", "5"]), "1.2.3-pre.4+build.5"),
            (v(1, 2, 3, &["pre", "4"], &["build"]), "1.2.3-pre.4+build"),
            (v(1, 2, 3, &[], &["build"]), "1.2.3+build"),
            (v(1, 2, 3, &["pre", "4"], &[]), "1.2.3-pre.4"),
            (v(1, 2, 3, &[], &["build", "4"]), "1.2.3+build.4"),
            (v(1, 2, 3, &[], &[]), "1.2.3"),
        ];

        for (input, out) in tests {
            assert_eq!(input.string(), out);
            assert_eq!(input.to_string(), out);
        }
    }

    #[test]
    fn test_version_compare() {
        let tests = [
            // https://semver.org/#spec-item-11
            // > Precedence is determined by the first difference when comparing each of these
            // > identifiers from left to right as follows: Major, minor, and patch versions are
            // > always compared numerically.
            ("1.0.0", "2.0.0", COMPARISON_LESS_THAN),
            ("1.0.0", "1.1.0", COMPARISON_LESS_THAN),
            ("1.0.0", "1.0.1", COMPARISON_LESS_THAN),
            ("2.0.0", "1.0.0", COMPARISON_GREATER_THAN),
            ("1.1.0", "1.0.0", COMPARISON_GREATER_THAN),
            ("1.0.1", "1.0.0", COMPARISON_GREATER_THAN),
            ("1.0.0", "1.0.0", COMPARISON_EQUAL_TO),
            // https://semver.org/#spec-item-11
            // > When major, minor, and patch are equal, a pre-release version has lower
            // > precedence than a normal version.
            ("1.0.0", "1.0.0-pre", COMPARISON_GREATER_THAN),
            ("1.0.1-pre", "1.0.0", COMPARISON_GREATER_THAN),
            ("1.0.0-pre", "1.0.0", COMPARISON_LESS_THAN),
            // https://semver.org/#spec-item-11
            // > identifiers consisting of only digits are compared numerically
            ("1.0.0-0", "1.0.0-1", COMPARISON_LESS_THAN),
            ("1.0.0-1", "1.0.0-0", COMPARISON_GREATER_THAN),
            ("1.0.0-2", "1.0.0-10", COMPARISON_LESS_THAN),
            ("1.0.0-10", "1.0.0-2", COMPARISON_GREATER_THAN),
            ("1.0.0-0", "1.0.0-0", COMPARISON_EQUAL_TO),
            // https://semver.org/#spec-item-11
            // > identifiers with letters or hyphens are compared lexically in ASCII sort order.
            ("1.0.0-a", "1.0.0-b", COMPARISON_LESS_THAN),
            ("1.0.0-a-2", "1.0.0-a-10", COMPARISON_GREATER_THAN),
            ("1.0.0-b", "1.0.0-a", COMPARISON_GREATER_THAN),
            ("1.0.0-a", "1.0.0-a", COMPARISON_EQUAL_TO),
            ("1.0.0-A", "1.0.0-a", COMPARISON_LESS_THAN),
            // https://semver.org/#spec-item-11
            // > Numeric identifiers always have lower precedence than non-numeric identifiers.
            ("1.0.0-0", "1.0.0-alpha", COMPARISON_LESS_THAN),
            ("1.0.0-alpha", "1.0.0-0", COMPARISON_GREATER_THAN),
            ("1.0.0-0", "1.0.0-0", COMPARISON_EQUAL_TO),
            ("1.0.0-alpha", "1.0.0-alpha", COMPARISON_EQUAL_TO),
            // https://semver.org/#spec-item-11
            // > A larger set of pre-release fields has a higher precedence than a smaller set, if all
            // > of the preceding identifiers are equal.
            ("1.0.0-alpha", "1.0.0-alpha.0", COMPARISON_LESS_THAN),
            ("1.0.0-alpha.0", "1.0.0-alpha", COMPARISON_GREATER_THAN),
            // https://semver.org/#spec-item-11
            // > Precedence for two pre-release versions with the same major, minor, and patch version
            // > MUST be determined by comparing each dot separated identifier from left to right until
            // > a difference is found [...]
            ("1.0.0-a.0.b.1", "1.0.0-a.0.b.2", COMPARISON_LESS_THAN),
            ("1.0.0-a.0.b.1", "1.0.0-b.0.a.1", COMPARISON_LESS_THAN),
            ("1.0.0-a.0.b.2", "1.0.0-a.0.b.1", COMPARISON_GREATER_THAN),
            ("1.0.0-b.0.a.1", "1.0.0-a.0.b.1", COMPARISON_GREATER_THAN),
            // https://semver.org/#spec-item-11
            // > Build metadata does not figure into precedence
            ("1.0.0+build", "1.0.0", COMPARISON_EQUAL_TO),
            ("1.0.0+build.stuff", "1.0.0", COMPARISON_EQUAL_TO),
            ("1.0.0", "1.0.0+build", COMPARISON_EQUAL_TO),
            ("1.0.0+build", "1.0.0+stuff", COMPARISON_EQUAL_TO),
            // https://semver.org/#spec-item-11
            // Edge cases for numeric and lexical comparison of prerelease identifiers.
            ("1.0.0-alpha.99999", "1.0.0-alpha.100000", COMPARISON_LESS_THAN),
            ("1.0.0-alpha.beta", "1.0.0-alpha.alpha", COMPARISON_GREATER_THAN),
        ];

        for (s1, s2, want) in tests {
            let v1 = try_parse_version(s1).unwrap_or_else(|e| panic!("{s1}: {e}"));
            let v2 = try_parse_version(s2).unwrap_or_else(|e| panic!("{s2}: {e}"));
            assert_eq!(v1.compare(&v2), want, "{s1} <=> {s2}");
        }
    }

    #[test]
    fn test_parse_errors() {
        assert_eq!(try_parse_version("1.02").unwrap_err(), "Could not parse version string from \"1.02\"");
        assert_eq!(try_parse_version("1.2.3-").unwrap_err(), "Could not parse version string from \"1.2.3-\"");
        assert_eq!(try_parse_version("1.2.3-a..b").unwrap_err(), "Could not parse version string from \"1.2.3-a..b\"");
        assert_eq!(
            try_parse_version("99999999999").unwrap_err(),
            "strconv.ParseUint: parsing \"99999999999\": value out of range"
        );
        assert_eq!(try_parse_version("1.2").unwrap(), v(1, 2, 0, &[], &[]));
        assert_eq!(try_parse_version("1.2.3-\u{212A}").unwrap(), v(1, 2, 3, &["\u{212A}"], &[]));
    }

    fn assert_version(a: &Version, b: &Version) {
        assert_eq!(a.major, b.major);
        assert_eq!(a.minor, b.minor);
        assert_eq!(a.patch, b.patch);
        assert_eq!(a.prerelease, b.prerelease);
        assert_eq!(a.build, b.build);
    }
}
