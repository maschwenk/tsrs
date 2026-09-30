// internal/testutil/baseline (comparison semantics of writeComparison) plus result classification.

use std::sync::LazyLock;

use regex::Regex;

pub const NO_CONTENT: &str = "<no content>";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Class {
    Pass,
    Codes,
    Fail,
    Crash,
    Timeout,
    Skip,
}

impl Class {
    pub const ALL: [Class; 6] = [Class::Pass, Class::Codes, Class::Fail, Class::Crash, Class::Timeout, Class::Skip];

    pub fn as_str(self) -> &'static str {
        match self {
            Class::Pass => "pass",
            Class::Codes => "codes",
            Class::Fail => "fail",
            Class::Crash => "crash",
            Class::Timeout => "timeout",
            Class::Skip => "skip",
        }
    }

    pub fn parse(s: &str) -> Option<Class> {
        Class::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

// writeComparison: the baseline passes when the generated content equals the reference (a missing
// reference reads as NoContent), except that a reference file that exists must not be matched by NoContent.
pub fn baseline_matches(expected: Option<&str>, actual: &str) -> bool {
    let found_expected = expected.is_some();
    let expected = expected.unwrap_or(NO_CONTENT);
    expected == actual && !(actual == NO_CONTENT && found_expected)
}

static HEADER_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:(\S.*?)\(([0-9]+),([0-9]+)\): )?(error|warning|suggestion|message) TS([0-9]+): ").unwrap());
static PRETTY_HEADER_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:(\S.*?):([0-9]+):([0-9]+) - )?(error|warning|suggestion|message) TS([0-9]+): ").unwrap());
static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new("\u{1b}\\[[0-9;]*m").unwrap());

// The multiset of (file, line, col, category, code) from the summary lines at the top of an error baseline.
pub fn diagnostic_keys(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    if text == NO_CONTENT {
        return keys;
    }
    for line in text.split("\r\n") {
        if line.starts_with("==== ") {
            break;
        }
        let (re, line) = if line.contains('\u{1b}') { (&*PRETTY_HEADER_LINE, ANSI.replace_all(line, "")) } else { (&*HEADER_LINE, line.into()) };
        if let Some(c) = re.captures(&line) {
            keys.push(format!(
                "{}({},{}) {} TS{}",
                c.get(1).map_or("", |m| m.as_str()),
                c.get(2).map_or("", |m| m.as_str()),
                c.get(3).map_or("", |m| m.as_str()),
                &c[4],
                &c[5]
            ));
        }
    }
    keys.sort();
    keys
}

pub fn classify(expected: Option<&str>, actual: &str) -> Class {
    if baseline_matches(expected, actual) {
        return Class::Pass;
    }
    if diagnostic_keys(expected.unwrap_or(NO_CONTENT)) == diagnostic_keys(actual) {
        return Class::Codes;
    }
    Class::Fail
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
}

// A one-line description of the first differing line, for summaries.
pub fn first_difference(expected: Option<&str>, actual: &str) -> String {
    let expected = expected.unwrap_or(NO_CONTENT);
    let e: Vec<&str> = expected.split('\n').collect();
    let a: Vec<&str> = actual.split('\n').collect();
    for i in 0..e.len().max(a.len()) {
        let el = e.get(i).map(|l| l.trim_end_matches('\r'));
        let al = a.get(i).map(|l| l.trim_end_matches('\r'));
        if el != al {
            return format!(
                "L{}: -{} +{}",
                i + 1,
                el.map_or("<eof>".to_string(), |l| truncate(l, 120)),
                al.map_or("<eof>".to_string(), |l| truncate(l, 120))
            );
        }
    }
    "line endings differ".to_string()
}

pub fn unified_diff(expected: Option<&str>, actual: &str, name: &str) -> String {
    let expected = expected.unwrap_or(NO_CONTENT);
    let exp = expected.replace("\r\n", "\n");
    let act = actual.replace("\r\n", "\n");
    let diff = similar::TextDiff::from_lines(&exp, &act);
    diff.unified_diff().context_radius(3).header(&format!("expected/{name}"), &format!("actual/{name}")).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_classes() {
        let exp = "a.ts(1,5): error TS2322: Type 'x' is not assignable.\r\n  chained line\r\nerror TS5023: Unknown.\r\n\r\n\r\n==== a.ts (1 errors) ====\r\n    let x";
        let act = "a.ts(1,5): error TS2322: Type 'y' is not assignable.\r\nerror TS5023: Unknown.\r\n\r\n\r\n==== a.ts (1 errors) ====\r\n    let x";
        assert_eq!(diagnostic_keys(exp), vec!["(,) error TS5023".to_string(), "a.ts(1,5) error TS2322".to_string()]);
        assert_eq!(classify(Some(exp), act), Class::Codes);
        assert_eq!(classify(Some(exp), exp), Class::Pass);
        assert_eq!(classify(None, NO_CONTENT), Class::Pass);
        assert_eq!(classify(Some(NO_CONTENT), NO_CONTENT), Class::Codes);
        assert_eq!(classify(None, act), Class::Fail);
    }
}
