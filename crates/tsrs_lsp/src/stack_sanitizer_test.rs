use crate::stack_sanitizer::sanitize_stack_trace;

// The Go tests' inputs are the "Unsanitized input" sections of their reference baselines
// (testdata/baselines/reference/lsp/stackSanitizer/); the whole baseline is compared like baseline.Run does.
fn reference_baseline(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ts-ref/tsc/testdata/baselines/reference/lsp/stackSanitizer").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("reading {}: {}", path.display(), err))
}

fn baseline_input(baseline: &str) -> String {
    let start = baseline.find("# Unsanitized input:\n\n````\n").unwrap() + "# Unsanitized input:\n\n````\n".len();
    let end = baseline.find("\n````\n\n# Sanitized output:").unwrap();
    baseline[start..end].to_string()
}

// stack_sanitizer_test.go:107
fn sanitized_stack_trace_baseline_contents(test_name: &str, input: &str, output: &str) -> String {
    let mut builder = String::new();
    builder.push_str("Test name: `");
    builder.push_str(test_name);
    builder.push_str("`\n\n# Unsanitized input:\n\n````\n");
    builder.push_str(input);
    builder.push_str("\n````\n\n# Sanitized output:\n\n````\n");
    builder.push_str(output);
    builder.push_str("\n````\n");
    builder
}

fn run_baseline(test_name: &str, baseline_name: &str) -> String {
    let expected = reference_baseline(baseline_name);
    let input = baseline_input(&expected);
    let output = sanitize_stack_trace(&input);
    let actual = sanitized_stack_trace_baseline_contents(test_name, &input, &output);
    assert_eq!(actual, expected, "baseline {} differs", baseline_name);
    output
}

// stack_sanitizer_test.go:13
#[test]
fn test_sanitized_debug_stack_trace_completions_request() {
    run_baseline("TestSanitizedDebugStackTraceCompletionsRequest", "completionsDebugStackTrace.md");
}

// stack_sanitizer_test.go:52
#[test]
fn test_sanitized_release_stack_trace_completions_request() {
    run_baseline("TestSanitizedReleaseStackTraceCompletionsRequest", "completionsReleaseStackTrace.md");
}

// stack_sanitizer_test.go:124: `(?i)(key|token|sig|secret|signature|password|passwd|pwd|android:value)[^a-zA-Z0-9]`.
fn vscode_generic_secret_regex_find(s: &str) -> Option<(usize, usize)> {
    const KEYWORDS: [&str; 9] = ["key", "token", "sig", "secret", "signature", "password", "passwd", "pwd", "android:value"];
    let b = s.as_bytes();
    for i in 0..b.len() {
        for keyword in KEYWORDS {
            let end = i + keyword.len();
            if end < b.len() && b[i..end].eq_ignore_ascii_case(keyword.as_bytes()) && !b[end].is_ascii_alphanumeric() {
                return Some((i, end + 1));
            }
        }
    }
    None
}

// stack_sanitizer_test.go:126
#[test]
fn test_sanitized_stack_trace_defeats_vs_code_generic_secret_regex() {
    let output = run_baseline("TestSanitizedStackTraceDefeatsVSCodeGenericSecretRegex", "genericSecretWorkaround.md");
    if let Some((start, end)) = vscode_generic_secret_regex_find(&output) {
        panic!(
            "sanitized stack trace would be redacted by VS Code's Generic Secret regex at [{} {}]: {:?}\nfull output:\n{}",
            start,
            end,
            &output[start..end],
            output
        );
    }
}
