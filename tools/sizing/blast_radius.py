#!/usr/bin/env python3
"""TOOL: the blast-radius counts of notes/mem-compact-ast-sizing.md section 6. Counts the lines per crate that match
each pattern (comment lines and test files excluded); `generated.rs` is counted apart, it is regenerated rather than
edited.

    python3 tools/sizing/blast_radius.py [repo root]
"""
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
CRATES = ["ast", "parser", "binder", "checker", "compiler", "declarations", "transformers", "printer", "ls", "lsp", "api",
          "api_codec", "astnav", "pseudochecker", "fourslash", "incremental", "execute", "project"]
GENERATED = ("generated.rs",)

PATTERNS = {
    "B: property-access parents": r"Kind::PropertyAccessExpression\b|is_property_access_expression\(|as_property_access_expression\(|is_property_access_entity_name_expression|is_access_expression\(|is_property_access_or_qualified_name",
    "B: member-declaration parents": r"Kind::(PropertySignature|PropertyDeclaration|MethodSignature|MethodDeclaration|GetAccessor|SetAccessor|EnumMember)\b|is_(property_signature|property_declaration|method_signature|method_declaration|get_accessor_declaration|set_accessor_declaration|enum_member|accessor|class_element|type_element|object_literal_element)\(|as_(property_signature_declaration|property_declaration|method_signature_declaration|method_declaration|get_accessor_declaration|set_accessor_declaration|enum_member)\(",
    "B: specifier parents": r"Kind::(ImportSpecifier|ExportSpecifier)\b|is_(import|export)_specifier\(|as_(import|export)_specifier\(|is_import_or_export_specifier\(",
    "B: generic name() reads": r"\.name\(\)",
    "B: property_name() reads": r"\.property_name\(\)|property_name_or_name\(",
    "B: name helpers (get_name_of_declaration, property-name text, declaration_name_to_string)": r"get_name_of_declaration\(|get_property_name_for_property_name_node\(|get_text_of_property_name\(|get_property_name_from_property_name\(|declaration_name_to_string\(|get_name_from_property_name\(",
    "C: NodeList / ModifierList type mentions": r"\bNodeList\b|\bModifierList\b",
    "C: list accessors returning P<NodeList>": r"\.(parameter_list|argument_list|type_argument_list|type_parameter_list|member_list|statement_list|comment_list|property_list|element_list)\(\)",
    "C: list factories": r"new_node_list(_from_slice|_from_static)?\(|new_modifier_list(_from_slice)?\(|new_lazy_node_list\(",
    "C: list positions (loc/pos/end of a list, trailing comma)": r"has_trailing_comma\(\)|_list\(\)[^;\n]*\.(loc|pos|end)\(\)|\.list\.loc|modifiers\(\)[^;\n]*\.(loc|pos|end)\(\)",
    "C: list visitors": r"\bvisit_nodes\(|\bvisit_node_list\(|\bvisit_modifiers\(",
}


def count(pat):
    rx = re.compile(pat)
    per = defaultdict(int)
    gen = 0
    for c in CRATES:
        d = ROOT / "crates" / f"tsrs_{c}"
        if not d.exists():
            continue
        for f in d.rglob("*.rs"):
            if "/tests/" in str(f) or f.name.endswith("_test.rs") or f.name == "sizing.rs":
                continue  # sizing.rs: the counting tool itself
            try:
                text = f.read_text()
            except UnicodeDecodeError:
                continue
            n = sum(1 for line in text.splitlines() if rx.search(line) and not line.lstrip().startswith("//"))
            if f.name in GENERATED:
                gen += n
            else:
                per[c] += n
    return per, gen


for name, pat in PATTERNS.items():
    per, gen = count(pat)
    tot = sum(per.values())
    parts = ", ".join(f"{c} {n}" for c, n in sorted(per.items(), key=lambda kv: -kv[1]) if n)
    print(f"{name}: {tot} hand-written lines (+{gen} generated) | {parts}")
