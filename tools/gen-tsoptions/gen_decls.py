#!/usr/bin/env python3
"""Converts the Go option declaration lists in ts-ref/tsc/internal/tsoptions/decls*.go into Rust statics.

Usage: gen_decls.py <go file> <rust file>
Only the `[]*CommandLineOption{...}` and `CommandLineOption{...}` literals are converted; the
surrounding Rust (imports, functions) lives in the hand-written part of the target file between the
`// BEGIN GENERATED` / `// END GENERATED` markers.
"""
import re
import sys

KIND = {
    "CommandLineOptionTypeString": "String",
    "CommandLineOptionTypeNumber": "Number",
    "CommandLineOptionTypeBoolean": "Boolean",
    "CommandLineOptionTypeObject": "Object",
    "CommandLineOptionTypeList": "List",
    "CommandLineOptionTypeListOrElement": "ListOrElement",
    "CommandLineOptionTypeEnum": "Enum",
    '"string"': "String",
    '"number"': "Number",
    '"boolean"': "Boolean",
    '"object"': "Object",
    '"list"': "List",
    '"enum"': "Enum",
}

CORE_ENUMS = ["ScriptTarget", "WatchFileKind", "WatchDirectoryKind", "PollingKind"]


def snake(name):
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    return s.lower()


def tristate(v):
    return {"core.TSUnknown": "Tristate::Unknown", "core.TSTrue": "Tristate::True", "core.TSFalse": "Tristate::False"}[v]


def default_value(v):
    if v in ("true", "false"):
        return f"DefaultValueDescription::Bool({v})"
    if v == "nil":
        return "DefaultValueDescription::None"
    if v.startswith('"'):
        return f"DefaultValueDescription::Str({v})"
    if re.fullmatch(r"-?\d+", v):
        return f"DefaultValueDescription::Int({v})"
    if v.startswith("diagnostics."):
        return f"DefaultValueDescription::Message(&diagnostics::{v[len('diagnostics.'):]})"
    if v.startswith("core.TS"):
        return f"DefaultValueDescription::Tristate({tristate(v)})"
    for e in CORE_ENUMS:
        if v.startswith("core." + e):
            return f"DefaultValueDescription::{e}({e}::{v[len('core.' + e):]})"
    raise Exception("unknown default value " + v)


def convert_field(key, value):
    if key == "Name":
        return f"name: {value}"
    if key == "ShortName":
        return f"short_name: {value}"
    if key == "Kind":
        return f"kind: CommandLineOptionKind::{KIND[value]}"
    if key in ("Category", "Description"):
        assert value.startswith("diagnostics."), value
        return f"{snake(key)}: Some(&diagnostics::{value[len('diagnostics.'):]})"
    if key == "DefaultValueDescription":
        return f"default_value_description: {default_value(value)}"
    if key == "transpileOptionValue":
        return f"transpile_option_value: {tristate(value)}"
    if key == "extraValidation":
        return "extra_validation: ExtraValidation::" + value[len("extraValidation"):]
    if key == "minValue":
        return f"min_value: {value}"
    if value in ("true", "false"):
        if key == "IsTSConfigOnly":
            return f"is_tsconfig_only: {value}"
        return f"{snake(key)}: {value}"
    raise Exception(f"unknown field {key}: {value}")


def convert_literal(body, indent):
    fields = []
    for line in body.split("\n"):
        line = re.sub(r"//.*$", "", line).strip()
        if not line:
            continue
        m = re.fullmatch(r"(\w+):\s*(.+?),?", line)
        if not m:
            raise Exception("cannot parse line: " + line)
        fields.append(convert_field(m.group(1), m.group(2)))
    pad = " " * (indent + 4)
    out = "CommandLineOption {\n"
    for f in fields:
        out += f"{pad}{f},\n"
    out += f"{pad}..CommandLineOption::DEFAULT\n"
    out += " " * indent + "}"
    return out


def scream(n):
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", n)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    return s.upper()


def main():
    go = open(sys.argv[1]).read()
    out = []
    # single named literals: var X = CommandLineOption{ ... }
    for m in re.finditer(r"^var (\w+) = CommandLineOption\{\n(.*?)\n\}", go, re.S | re.M):
        name, body = m.group(1), m.group(2)
        vis = "pub " if name[0].isupper() else "pub(crate) "
        name = scream(name)
        out.append(f"{vis}static {name}: CommandLineOption = {convert_literal(body, 0)};\n")
    for m in re.finditer(r"^var (\w+) = \[\]\*CommandLineOption\{\n(.*?)\n\}\n", go, re.S | re.M):
        name, body = m.group(1), m.group(2)
        vis = "pub " if name[0].isupper() else "pub(crate) "
        name = scream(name)
        items = []
        # split top-level elements
        depth = 0
        cur = None
        for line in body.split("\n"):
            stripped = re.sub(r"//.*$", "", line).strip()
            if depth == 0:
                if stripped == "{":
                    depth = 1
                    cur = []
                    continue
                if stripped.startswith("&"):
                    items.append(("ref", stripped[1:].rstrip(",")))
                    continue
                if stripped == "":
                    continue
                raise Exception("unexpected top-level line: " + line)
            else:
                if stripped in ("},", "}"):
                    depth = 0
                    items.append(("lit", "\n".join(cur)))
                    cur = None
                    continue
                cur.append(line)
        refs = [i for i in items if i[0] == "ref"]
        lits = [i for i in items if i[0] == "lit"]
        # Literal elements go into a static array (stable addresses); the list itself is a
        # LazyLock<Vec<&'static CommandLineOption>> preserving the Go order, including &Named refs.
        arr = f"{name}_ITEMS"
        s = f"static {arr}: [CommandLineOption; {len(lits)}] = [\n"
        for _, body in lits:
            s += "    " + convert_literal(body, 4) + ",\n"
        s += "];\n\n"
        s += f"{vis}static {name}: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {{\n"
        s += "    vec![\n"
        li = 0
        for kind, v in items:
            if kind == "ref":
                s += f"        &{scream(v)},\n"
            else:
                s += f"        &{arr}[{li}],\n"
                li += 1
        s += "    ]\n});\n"
        out.append(s)
    text = "\n".join(out)
    target = open(sys.argv[2]).read()
    begin = target.index("// BEGIN GENERATED\n") + len("// BEGIN GENERATED\n")
    end = target.index("// END GENERATED")
    open(sys.argv[2], "w").write(target[:begin] + text + target[end:])


main()
