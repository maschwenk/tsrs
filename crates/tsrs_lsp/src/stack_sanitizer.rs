// VS Code's telemetry pipeline redacts any string matching
// /(key|token|sig|secret|signature|password|passwd|pwd|android:value)[^a-zA-Z0-9]/i
// as `<REDACTED: Generic Secret>`, which trips on innocuous Go frames like
// `getSignatureHelp(`. Insert `X_X` after each trigger keyword that we know
// can appear in our sanitized output, when followed by punctuation we
// actually emit (`(`, `[`, `.`, `|`); reverse by removing the marker (replace
// `X_X` with the empty string) on the dashboard.
//
// stack_sanitizer.go:17: `(?i)(key|token|signature|sig|pwd)([(\[.|])`, matched by hand (leftmost-first
// alternation, non-overlapping matches, like Go's regexp.ReplaceAllString).
const GENERIC_SECRET_KEYWORDS: [&str; 5] = ["key", "token", "signature", "sig", "pwd"];

fn match_generic_secret_keyword_at(s: &[u8], i: usize) -> Option<usize> {
    for keyword in GENERIC_SECRET_KEYWORDS {
        let end = i + keyword.len();
        if end < s.len() && s[i..end].eq_ignore_ascii_case(keyword.as_bytes()) && matches!(s[end], b'(' | b'[' | b'.' | b'|') {
            return Some(keyword.len());
        }
    }
    None
}

// stack_sanitizer.go:19
pub(crate) fn defeat_generic_secret_regex(s: &str) -> String {
    let b = s.as_bytes();
    let mut result = String::with_capacity(s.len());
    let mut last = 0;
    let mut i = 0;
    while i < b.len() {
        if let Some(n) = match_generic_secret_keyword_at(b, i) {
            result.push_str(&s[last..i + n]);
            result.push_str("X_X");
            result.push(b[i + n] as char);
            i += n + 1;
            last = i;
            continue;
        }
        i += 1;
    }
    result.push_str(&s[last..]);
    result
}

// stack_sanitizer.go:23
pub(crate) fn sanitize_stack_trace(stack: &str) -> String {
    // TODO: should we just look for the first '(' and
    // just strip everything before the prior newline?
    let Some(start_index) = stack.find("runtime/debug.Stack()") else {
        return String::new();
    };
    let stack = &stack[start_index..];

    let mut result = String::new();

    for (line_num, line) in stack.split_inclusive('\n').enumerate() {
        if line_num > 0 {
            result.push('\n');
        }

        let b = line.as_bytes();
        let mut i = 0;
        // Skip whitespace
        while i < b.len() {
            if b[i] != b' ' && b[i] != b'\t' {
                break;
            }
            i += 1;
        }

        result.push_str(&line[..i]);

        let line = &line[i..];

        if let Some(our_module_index) = line.find("TypeScript/tsc/") {
            let line = &line[our_module_index..];
            write_sanitized_module_or_path(line, &mut result);
        } else {
            result.push_str("(REDACTED FRAME)");
        }
    }

    defeat_generic_secret_regex(&result)
}

// stack_sanitizer.go:64
fn write_sanitized_module_or_path(line: &str, result: &mut String) {
    // We don't expect things like \r, but it doesn't hurt to trim just in case.
    let mut line = line.trim();

    if let Some(plus_hex) = line.find(" +0x") {
        line = &line[..plus_hex];
    } else if let Some(in_goroutine) = line.rfind(" in goroutine ") {
        line = &line[..in_goroutine];
    }

    for (segment_index, segment) in line.split('/').enumerate() {
        if segment_index > 0 {
            result.push_str("|>");
        }

        // See if the string ends with ), and strip out all the arguments.
        if segment.ends_with(')') {
            let Some(open_paren_index) = segment.rfind('(') else {
                // Closing parenthesis, but no opening - bail out.
                result.push_str("???");
                continue;
            };

            result.push_str(&segment[..open_paren_index]);
            result.push_str("()");
            continue;
        }

        result.push_str(segment);
    }
}
