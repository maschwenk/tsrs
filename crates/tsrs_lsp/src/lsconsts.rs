// The `ls` package values the server's capabilities need (ls/completions.go, ls/signaturehelp.go,
// ls/semantictokens.go). Completions and signature help are ported (re-exported from tsrs_ls); semantic tokens
// are phase 3, so their values live here until semantictokens.go is ported.

use tsrs_lsproto as lsproto;
use tsrs_lsproto::{SemanticTokenModifier, SemanticTokenType};

pub(crate) use tsrs_ls::{COMPLETION_TRIGGER_CHARACTERS, SIGNATURE_HELP_RETRIGGER_CHARACTERS, SIGNATURE_HELP_TRIGGER_CHARACTERS};

// semantictokens.go:21
// tokenTypes defines the order of token types for encoding
const TOKEN_TYPES: [SemanticTokenType; 23] = [
    SemanticTokenType::Namespace,
    SemanticTokenType::Class,
    SemanticTokenType::Enum,
    SemanticTokenType::Interface,
    SemanticTokenType::Struct,
    SemanticTokenType::TypeParameter,
    SemanticTokenType::Type,
    SemanticTokenType::Parameter,
    SemanticTokenType::Variable,
    SemanticTokenType::Property,
    SemanticTokenType::EnumMember,
    SemanticTokenType::Decorator,
    SemanticTokenType::Event,
    SemanticTokenType::Function,
    SemanticTokenType::Method,
    SemanticTokenType::Macro,
    SemanticTokenType::Label,
    SemanticTokenType::Comment,
    SemanticTokenType::String,
    SemanticTokenType::Keyword,
    SemanticTokenType::Number,
    SemanticTokenType::Regexp,
    SemanticTokenType::Operator,
];

// semantictokens.go:47
// tokenModifiers defines the order of token modifiers for encoding
const TOKEN_MODIFIERS: [SemanticTokenModifier; 11] = [
    SemanticTokenModifier::Declaration,
    SemanticTokenModifier::Definition,
    SemanticTokenModifier::Readonly,
    SemanticTokenModifier::Static,
    SemanticTokenModifier::Deprecated,
    SemanticTokenModifier::Abstract,
    SemanticTokenModifier::Async,
    SemanticTokenModifier::Modification,
    SemanticTokenModifier::Documentation,
    SemanticTokenModifier::DefaultLibrary,
    SemanticTokenModifier("local"),
];

// semantictokens.go:109
// SemanticTokensLegend returns the legend describing the token types and modifiers.
// It filters the legend to only include types and modifiers that the client supports,
// as indicated by clientCapabilities.
pub(crate) fn semantic_tokens_legend(client_capabilities: &lsproto::ResolvedSemanticTokensClientCapabilities) -> lsproto::SemanticTokensLegend {
    let mut types = Vec::with_capacity(TOKEN_TYPES.len());
    for t in TOKEN_TYPES {
        if client_capabilities.token_types.iter().any(|c| c == t.0) {
            types.push(t.0.to_string());
        }
    }
    let mut modifiers = Vec::with_capacity(TOKEN_MODIFIERS.len());
    for m in TOKEN_MODIFIERS {
        if client_capabilities.token_modifiers.iter().any(|c| c == m.0) {
            modifiers.push(m.0.to_string());
        }
    }
    lsproto::SemanticTokensLegend { token_types: types, token_modifiers: modifiers }
}

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}
