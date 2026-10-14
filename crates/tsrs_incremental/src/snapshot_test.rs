// The signature text of a declaration file with emit diagnostics (snapshot.go diagnosticToStringBuilder) is hashed into
// the build info, so it must match Go byte for byte.

use tsrs_ast::{new_diagnostic, SourceFileParseOptions};
use tsrs_core::{get_script_kind_from_file_name, TextRange};
use tsrs_core::tspath::Path;
use tsrs_diagnostics as diagnostics;

use crate::snapshot::diagnostic_to_string_builder;

#[test]
fn signature_diagnostic_prints_go_position_and_length() {
    let file_name = "/a.ts";
    let file = tsrs_parser::parse_source_file(
        SourceFileParseOptions { file_name: file_name.to_string(), path: Path::new(file_name.to_string()), external_module_indicator_options: tsrs_ast::parseoptions::ExternalModuleIndicatorOptions::default() },
        "const a = class { private p = 10; };",
        get_script_kind_from_file_name(file_name),
    );
    let diagnostic = new_diagnostic(
        Some(file),
        TextRange::new(6, 7),
        &diagnostics::Property_0_of_exported_anonymous_class_type_may_not_be_private_or_protected,
        &[&"p"],
    );
    let mut builder = String::new();
    diagnostic_to_string_builder(Some(diagnostic), file, &mut builder);
    assert_eq!(builder, "\n(6,1): error4094: Property_0_of_exported_anonymous_class_type_may_not_be_private_or_protected_4094\np\n");
}
