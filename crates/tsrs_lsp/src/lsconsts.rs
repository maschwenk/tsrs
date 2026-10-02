// The `ls` package values the server's capabilities need (ls/completions.go, ls/signaturehelp.go; ls/semantictokens.go's
// legend is now `tsrs_ls::semantic_tokens_legend`). Those Go files are ported in phase 3 (completions, signature help, semantic tokens);
// until then the values live here. Move them to tsrs_ls when the files are ported.


// completions.go:259
pub(crate) const COMPLETION_TRIGGER_CHARACTERS: [&str; 10] = [".", "\"", "'", "`", "/", "@", "<", "#", " ", "*"];

// signaturehelp.go:25
pub(crate) const SIGNATURE_HELP_TRIGGER_CHARACTERS: [&str; 3] = ["(", ",", "<"];
pub(crate) const SIGNATURE_HELP_RETRIGGER_CHARACTERS: [&str; 1] = [")"];

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}
