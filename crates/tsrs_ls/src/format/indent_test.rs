use tsrs_ast::{Kind, Node};
use tsrs_core::{ScriptKind, P};

use super::*;
use crate::astnav::parse_for_test;

// indent_test.go:13
#[test]
fn test_get_containing_list_named_imports() {
    let text = "import type {\n    AAA,\n    BBB,\n} from \"./bar\";";

    let source_file = parse_for_test("/test.ts", text, ScriptKind::TS);

    // Find ImportSpecifier nodes (AAA and BBB)
    let mut import_specifiers: Vec<P<Node>> = Vec::new();
    for_each_descendant_of_kind(source_file.as_node(), Kind::ImportSpecifier, &mut |node| import_specifiers.push(node));

    assert!(import_specifiers.len() == 2, "Expected 2 import specifiers, got {}", import_specifiers.len());

    // Test GetContainingList for each import specifier
    for specifier in import_specifiers {
        let list = get_containing_list(specifier, source_file);
        assert!(list.is_some(), "GetContainingList should return non-nil for import specifier");
        assert!(list.unwrap().nodes.len() == 2, "Expected list with 2 elements, got {}", list.unwrap().nodes.len());
    }
}

// indent_test.go:42
fn for_each_descendant_of_kind(node: P<Node>, kind: Kind, action: &mut dyn FnMut(P<Node>)) {
    node.for_each_child(&mut |child| {
        if child.kind() == kind {
            action(child);
        }
        for_each_descendant_of_kind(child, kind, action);
        false
    });
}
