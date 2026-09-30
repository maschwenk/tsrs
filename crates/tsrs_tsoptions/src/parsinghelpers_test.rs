use tsrs_core::CompilerOptions;

use crate::commandlineoption::CompilerOptionsValue;
use crate::parsinghelpers::{for_each_compiler_options_field, parse_compiler_options_worker};

#[test]
fn test_parse_compiler_option_no_missing_fields() {
    let mut missing_keys: Vec<&str> = Vec::new();
    macro_rules! check {
        ($($field:ident: $json:literal,)*) => {
            $(
                let mut co = CompilerOptions::default();
                // Go passes the field's zero value; a float is accepted by every value parser.
                if !parse_compiler_options_worker($json, &CompilerOptionsValue::Float(0.0), &mut co) {
                    missing_keys.push($json);
                }
            )*
        };
    }
    for_each_compiler_options_field!(check);
    assert!(missing_keys.is_empty(), "The following keys are missing entries in the ParseCompilerOptions switch statement:\n{missing_keys:?}");
}
