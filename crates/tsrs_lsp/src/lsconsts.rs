// The `ls` package values the server's capabilities need (ls/completions.go, ls/signaturehelp.go), re-exported from
// tsrs_ls; the semantic tokens legend is `tsrs_ls::semantic_tokens_legend`.

pub(crate) use tsrs_ls::{COMPLETION_TRIGGER_CHARACTERS, SIGNATURE_HELP_RETRIGGER_CHARACTERS, SIGNATURE_HELP_TRIGGER_CHARACTERS};

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}
