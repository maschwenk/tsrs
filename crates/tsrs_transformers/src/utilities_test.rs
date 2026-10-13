// utilities.rs against Go's behaviour on synthesized nodes (position -1).

use tsrs_ast::SourceFileParseOptions;
use tsrs_core::{get_script_kind_from_file_name, P};
use tsrs_core::tspath::Path;

use crate::is_original_node_single_line;

#[test]
fn a_synthesized_original_node_is_single_line() {
    let file_name = "/a.ts";
    let file = tsrs_parser::parse_source_file(
        SourceFileParseOptions { file_name: file_name.to_string(), path: Path::new(file_name.to_string()), external_module_indicator_options: Default::default() },
        "const a = 1;\nconst b = 2;\n",
        get_script_kind_from_file_name(file_name),
    );
    let emit_context = tsrs_printer::new_emit_context();
    // A node a transformer made and placed in the file: it has no original and no source position.
    let node: P<tsrs_ast::Node> = emit_context.factory.new_identifier("x");
    node.set_parent(Some(file.as_node()));
    // Go: GetECMALineOfPosition(file, -1) is -1 for both ends.
    assert!(is_original_node_single_line(emit_context, Some(node)));
}
