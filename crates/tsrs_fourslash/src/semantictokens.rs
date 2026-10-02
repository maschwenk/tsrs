use tsrs_ls::lsconv;
use tsrs_lsproto as lsproto;

use crate::fourslash::{new_test_converters, FourslashTest};
use crate::go::quote;
use crate::testing::T;

// semantictokens.go:12
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticToken {
    pub type_: String,
    pub text: String,
}

impl FourslashTest {
    // semantictokens.go:17
    pub fn verify_semantic_tokens(&mut self, t: &T, expected: &[SemanticToken]) {
        FourslashTest::server_unavailable(t, "feature not ported: semantic tokens (VerifySemanticTokens)")
    }
}

// semantictokens.go:55
pub(crate) fn decode_semantic_tokens(f: &FourslashTest, data: &[u32], token_types: &[String], token_modifiers: &[String]) -> Vec<SemanticToken> {
    if data.len() % 5 != 0 {
        panic!("Invalid semantic tokens data length: {}", data.len());
    }

    let script_info = f.get_script_info(&f.active_filename);
    let line_map = script_info.line_map.clone();
    let converters = new_test_converters(lsconv::new_converters(lsproto::PositionEncodingKind::UTF8, move |_| Some(line_map.clone())));

    let mut tokens = Vec::new();
    let mut prev_line: u32 = 0;
    let mut prev_char: u32 = 0;

    let mut i = 0;
    while i < data.len() {
        let delta_line = data[i];
        let delta_char = data[i + 1];
        let length = data[i + 2];
        let token_type_idx = data[i + 3];
        let token_modifier_mask = data[i + 4];

        // Calculate absolute position
        let line = prev_line + delta_line;
        let char = if delta_line == 0 { prev_char + delta_char } else { delta_char };

        // Get token type
        if token_type_idx as usize >= token_types.len() {
            panic!("Token type index out of range: {token_type_idx}");
        }
        let token_type = &token_types[token_type_idx as usize];

        // Get modifiers
        let mut modifiers: Vec<&str> = Vec::new();
        for (i, m) in token_modifiers.iter().enumerate() {
            if token_modifier_mask & (1 << i) != 0 {
                modifiers.push(m);
            }
        }

        // Build full type string (type.modifier1.modifier2)
        let mut type_str = token_type.clone();
        if !modifiers.is_empty() {
            type_str = type_str + "." + &modifiers.join(".");
        }

        // Get the text
        let start_pos = lsproto::Position { line, character: char };
        let end_pos = lsproto::Position { line, character: char + length };
        let start_offset = converters.line_and_character_to_position(script_info.clone(), start_pos) as usize;
        let end_offset = converters.line_and_character_to_position(script_info.clone(), end_pos) as usize;
        let text = script_info.content[start_offset..end_offset].to_string();

        tokens.push(SemanticToken { type_: type_str, text });

        prev_line = line;
        prev_char = char;
        i += 5;
    }

    tokens
}

// semantictokens.go:124
pub(crate) fn format_semantic_tokens(tokens: &[SemanticToken]) -> String {
    let mut lines = Vec::new();
    for (i, tok) in tokens.iter().enumerate() {
        lines.push(format!("  [{}] {{Type: {}, Text: {}}}", i, quote(&tok.type_), quote(&tok.text)));
    }
    lines.join("\n")
}
