#!/usr/bin/env python3
"""Converts the test tables of ts-ref/tsc/internal/tsoptions/tsconfigparsing_test.go into Rust data.

Usage: gen_tsconfig_tests.py <go test file> <rust test file>
Replaces the region between `// BEGIN GENERATED` / `// END GENERATED` in the Rust file.
"""
import re
import sys


def rust_raw(s):
    hashes = "#"
    while '"' + hashes in s:
        hashes += "#"
    return f"r{hashes}\"{s}\"{hashes}"


def convert_value_strings(text):
    # Replace Go backtick raw strings with Rust raw strings.
    out = []
    i = 0
    while i < len(text):
        c = text[i]
        if c == "`":
            j = text.index("`", i + 1)
            out.append(rust_raw(text[i + 1:j]))
            i = j + 1
        elif c == '"':
            j = i + 1
            while text[j] != '"':
                if text[j] == "\\":
                    j += 1
                j += 1
            out.append(text[i:j + 1])
            i = j + 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


def find_block(go, start_pattern):
    m = re.search(start_pattern, go, re.M)
    start = m.end() - 1  # at the opening brace
    depth = 0
    i = start
    in_str = None
    while True:
        c = go[i]
        if in_str:
            if c == "\\" and in_str == '"':
                i += 2
                continue
            if c == in_str:
                in_str = None
        elif c in "`\"":
            in_str = c
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return go[start:i + 1]
        i += 1


def convert_test_configs(block):
    # block: []testConfig{ {...}, {...} } content (with outer braces)
    s = convert_value_strings(block)
    s = re.sub(r"map\[string\]string\{", "&[", s)
    return s


def main():
    go = open(sys.argv[1]).read()
    out = []
    for name in ["tsconfigWithExtends", "tsconfigWithoutConfigDir", "tsconfigWithConfigDir", "tsconfigWithExtendsAndConfigDir"]:
        m = re.search(r"^var " + name + r" = `(.*?)`", go, re.S | re.M)
        const = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", name).upper()
        out.append(f"const {const}: &str = {rust_raw(m.group(1))};\n")

    block = find_block(go, r"^var parseJsonConfigFileTests = \[\]parseJsonConfigTestCase\{")
    s = convert_value_strings(block[1:-1])
    # map literals: map[string]string{ "k": v, ... } -> &[("k", v), ...]
    def conv_map(m):
        inner = m.group(1)
        return inner
    # Tokenize map bodies by finding 'map[string]string{' and matching braces.
    res = []
    i = 0
    while True:
        j = s.find("map[string]string{", i)
        if j < 0:
            res.append(s[i:])
            break
        res.append(s[i:j])
        k = j + len("map[string]string{")
        depth = 1
        m = k
        in_str = False
        raw_end = None
        while depth:
            if s.startswith('r#', m) or s.startswith('r"', m):
                # skip Rust raw string
                hm = re.match(r'r(#*)"', s[m:])
                term = '"' + hm.group(1)
                end = s.index(term, m + len(hm.group(0)))
                m = end + len(term)
                continue
            c = s[m]
            if c == '"':
                end = m + 1
                while s[end] != '"':
                    if s[end] == "\\":
                        end += 1
                    end += 1
                m = end + 1
                continue
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
            m += 1
        body = s[k:m - 1]
        # split entries "key": value,
        entries = []
        pos = 0
        entry_re = re.compile(r'\s*("(?:[^"\\]|\\.)*")\s*:\s*')
        while True:
            em = entry_re.match(body, pos)
            if not em:
                break
            key = em.group(1)
            vpos = em.end()
            if body.startswith("r", vpos) and re.match(r'r#*"', body[vpos:]):
                hm = re.match(r'r(#*)"', body[vpos:])
                term = '"' + hm.group(1)
                end = body.index(term, vpos + len(hm.group(0))) + len(term)
            elif body[vpos] == '"':
                end = vpos + 1
                while body[end] != '"':
                    if body[end] == "\\":
                        end += 1
                    end += 1
                end += 1
            else:
                end = vpos
                while end < len(body) and body[end] not in ",\n":
                    end += 1
            value = body[vpos:end].strip()
            value = re.sub(r"\btsconfig(\w+)", lambda mm: "TSCONFIG_" + re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", mm.group(1)).upper(), value)
            entries.append(f"({key}, {value})")
            pos = end
            cm = re.match(r"\s*,", body[pos:])
            if cm:
                pos += cm.end()
        res.append("&[" + ", ".join(entries) + "]")
        i = m
    s = "".join(res)
    s = s.replace("[]testConfig{", "vec![")
    # testConfig item braces: '{{' -> 'TestConfig {' ; '}}' -> '}]' handled with a small state machine below
    s = re.sub(r"\btitle:", "title:", s)
    s = re.sub(r"\bincludeCompilerOptions:", "include_compiler_options:", s)
    s = re.sub(r"\bjsonText:", "json_text:", s)
    s = re.sub(r"\bconfigFileName:", "config_file_name:", s)
    s = re.sub(r"\bbasePath:", "base_path:", s)
    s = re.sub(r"\ballFileList:", "all_file_list:", s)
    lines = []
    for line in s.split("\n"):
        rules = {
            "\t{": ["\tParseJsonConfigTestCase {"],
            "\t},": ["\t\t..Default::default()", "\t},"],
            "\t\tinput: vec![{": ["\t\tinput: vec![TestConfig {"],
            "\t\t}, {": ["\t\t\t..Default::default()", "\t\t}, TestConfig {"],
            "\t\t}},": ["\t\t\t..Default::default()", "\t\t}],"],
            "\t\t\t{": ["\t\t\tTestConfig {"],
            "\t\t\t},": ["\t\t\t\t..Default::default()", "\t\t\t},"],
            "\t\t},": ["\t\t],"],
        }
        if line in rules:
            lines.extend(rules[line])
        else:
            lines.append(line)
    table = "\n".join(lines)
    out.append("\nfn parse_json_config_file_tests() -> Vec<ParseJsonConfigTestCase> {\n    vec![\n" + table + "\n    ]\n}\n")
    target = open(sys.argv[2]).read()
    b = target.index("// BEGIN GENERATED\n") + len("// BEGIN GENERATED\n")
    e = target.index("// END GENERATED")
    open(sys.argv[2], "w").write(target[:b] + "".join(out) + target[e:])


main()
