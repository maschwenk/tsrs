#!/usr/bin/env python3
"""Builds resolver scenarios (JSON lines) from the TypeScript test corpus for tsrs-oracle-module.

Each test case's virtual files and a subset of its compiler options become a scenario; every
module specifier / types reference found in its files becomes a resolution request (in both
CommonJS and ESM mode), so the Go resolver and tsrs_module can be compared request by request.

usage: gen.py > scenarios.jsonl
"""
import json
import os
import re
import sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "ts-ref", "tsc", "testdata", "tests", "cases")
SRC = "/.src"

OPTION_RE = re.compile(r"^\/{2}\s*@(\w+)\s*:\s*([^\r\n]*)", re.M)
SPEC_RES = [
    re.compile(r"""\bfrom\s*['"]([^'"\n]+)['"]"""),
    re.compile(r"""^\s*import\s*['"]([^'"\n]+)['"]""", re.M),
    re.compile(r"""\bimport\s*\(\s*['"]([^'"\n]+)['"]"""),
    re.compile(r"""\brequire\s*\(\s*['"]([^'"\n]+)['"]"""),
    re.compile(r"""\bexport\s*\*\s*from\s*['"]([^'"\n]+)['"]"""),
    re.compile(r"""\bmodule\s+['"]([^'"\n]+)['"]"""),
]
TYPES_RE = re.compile(r"""///\s*<reference\s+types\s*=\s*['"]([^'"]+)['"]""")

MODULE_KINDS = {
    "none": 0, "commonjs": 1, "amd": 2, "umd": 3, "system": 4, "es6": 5, "es2015": 5, "es2020": 6, "es2022": 7,
    "esnext": 99, "node16": 100, "node18": 101, "node20": 102, "nodenext": 199, "preserve": 200,
}
RESOLUTION_KINDS = {"classic": 1, "node": 2, "node10": 2, "node16": 3, "nodenext": 99, "bundler": 100}
JSX = {"none": 0, "preserve": 1, "react": 2, "react-native": 3, "react-jsx": 4, "react-jsxdev": 5}
BOOL_OPTIONS = {
    "resolvejsonmodule": "resolveJsonModule",
    "allowjs": "allowJs",
    "nodtsresolution": "noDtsResolution",
    "preservesymlinks": "preserveSymlinks",
    "resolvepackagejsonexports": "resolvePackageJsonExports",
    "resolvepackagejsonimports": "resolvePackageJsonImports",
    "allowarbitraryextensions": "allowArbitraryExtensions",
}
LIST_OPTIONS = {"customconditions": "customConditions", "modulesuffixes": "moduleSuffixes", "rootdirs": "rootDirs", "typeroots": "typeRoots", "types": "types"}
PATH_OPTIONS = {"outdir": "outDir", "declarationdir": "declarationDir", "rootdir": "rootDir"}
PATH_LIST_OPTIONS = {"rootDirs", "typeRoots"}

INTERESTING = re.compile(r"node_modules|@moduleResolution|package\.json|@traceResolution|@paths|rootDirs|typeRoots|reference types|@types|customConditions|moduleSuffixes|@link", re.I)


def normalize(path):
    parts = []
    root = "/" if path.startswith("/") else ""
    for p in path.split("/"):
        if p in ("", "."):
            continue
        if p == "..":
            if parts:
                parts.pop()
            continue
        parts.append(p)
    return root + "/".join(parts)


def absolute(path, base):
    if path.startswith("/"):
        return normalize(path)
    return normalize(base + "/" + path)


def strip_json_comments(text):
    out, i, n, in_str = [], 0, len(text), False
    while i < n:
        c = text[i]
        if in_str:
            out.append(c)
            if c == "\\" and i + 1 < n:
                out.append(text[i + 1])
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            out.append(c)
            i += 1
        elif text.startswith("//", i):
            while i < n and text[i] != "\n":
                i += 1
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
        else:
            out.append(c)
            i += 1
    return re.sub(r",(\s*[}\]])", r"\1", "".join(out))


def first_variant(value):
    return value.split(",")[0].strip().lower()


def variants(value, table):
    result = []
    for v in value.split(","):
        v = v.strip().lower()
        if v in table and table[v] not in result:
            result.append(table[v])
    return result[:3]


def apply_option(opts, key, value, base, variant_values):
    lk = key.lower()
    if lk in BOOL_OPTIONS:
        v = first_variant(value)
        if v in ("true", "false"):
            opts[BOOL_OPTIONS[lk]] = v == "true"
    elif lk in LIST_OPTIONS:
        items = value if isinstance(value, list) else [x.strip() for x in value.split(",") if x.strip()]
        name = LIST_OPTIONS[lk]
        if name in PATH_LIST_OPTIONS:
            items = [absolute(x, base) for x in items]
        opts[name] = items
    elif lk in PATH_OPTIONS:
        opts[PATH_OPTIONS[lk]] = absolute(value, base)
    elif lk == "jsx":
        v = first_variant(value) if isinstance(value, str) else value
        if v in JSX:
            opts["jsx"] = JSX[v]
    elif lk == "paths":
        if isinstance(value, str):
            try:
                value = json.loads(value)
            except Exception:
                return
        if isinstance(value, dict):
            opts["paths"] = [[k, v] for k, v in value.items() if isinstance(v, list)]
            opts.setdefault("pathsBasePath", base)
    elif lk == "baseurl":
        opts["pathsBasePath"] = absolute(value, base)
    elif lk == "module":
        variant_values["module"] = variants(value, MODULE_KINDS) if isinstance(value, str) else []
    elif lk == "moduleresolution":
        variant_values["moduleResolution"] = variants(value, RESOLUTION_KINDS) if isinstance(value, str) else []


def parse_test(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        content = f.read()
    settings = {}
    for m in OPTION_RE.finditer(content):
        settings[m.group(1).lower()] = m.group(2).strip().rstrip(";")
    cwd = settings.get("currentdirectory", SRC)
    case_sensitive = settings.get("usecasesensitivefilenames", "true").lower() != "false"

    files = {}
    links = []
    name = os.path.basename(path)
    current, lines = name, []
    for line in content.split("\n"):
        m = re.match(r"^\/{2}\s*@filename\s*:\s*([^\r\n]*)", line, re.I)
        if m:
            if lines or current != name:
                files[current] = "\n".join(lines)
            current, lines = m.group(1).strip(), []
            continue
        m = re.match(r"^\/{2}\s*@link\s*:\s*(.+?)\s*->\s*(.+?)\s*$", line, re.I)
        if m:
            links.append((m.group(1), m.group(2)))
            continue
        if OPTION_RE.match(line):
            continue
        lines.append(line)
    files[current] = "\n".join(lines)
    files = {absolute(k, cwd): v for k, v in files.items()}

    opts = {}
    variant_values = {}
    for key, value in settings.items():
        apply_option(opts, key, value, cwd, variant_values)

    tsconfigs = [p for p in files if os.path.basename(p) == "tsconfig.json"]
    if tsconfigs:
        config_path = sorted(tsconfigs, key=len)[0]
        try:
            config = json.loads(strip_json_comments(files[config_path]))
            compiler_options = config.get("compilerOptions", {}) or {}
            base = os.path.dirname(config_path)
            for key, value in compiler_options.items():
                if isinstance(value, bool):
                    value = "true" if value else "false"
                apply_option(opts, key, value, base, variant_values)
            opts["configFilePath"] = config_path
        except Exception:
            pass

    json_files = {k: v for k, v in files.items()}
    for target, link in links:
        json_files[absolute(link, cwd)] = {"symlink": absolute(target, cwd)}

    requests = []
    seen = set()
    for file_name, text in files.items():
        if not re.search(r"\.(d\.)?[mc]?[jt]sx?$", file_name):
            continue
        for regex in SPEC_RES:
            for m in regex.finditer(text):
                spec = m.group(1)
                for mode in (1, 99):
                    key = ("module", spec, file_name, mode)
                    if key not in seen:
                        seen.add(key)
                        requests.append({"kind": "module", "name": spec, "file": file_name, "mode": mode})
        for m in TYPES_RE.finditer(text):
            spec = m.group(1)
            for mode in (1, 99):
                key = ("type", spec, file_name, mode)
                if key not in seen:
                    seen.add(key)
                    requests.append({"kind": "type", "name": spec, "file": file_name, "mode": mode})
    for t in opts.get("types", []):
        for mode in (1, 99):
            requests.append({"kind": "type", "name": t, "file": cwd + "/__inferred type names__.ts", "mode": mode})
    requests = requests[:80]
    if not requests:
        return []

    scenarios = []
    modules = variant_values.get("module") or [None]
    resolutions = variant_values.get("moduleResolution") or [None]
    for module in modules:
        for resolution in resolutions:
            o = dict(opts)
            if module is not None:
                o["module"] = module
            if resolution is not None:
                o["moduleResolution"] = resolution
            label = os.path.relpath(path, ROOT) + f"[module={module},moduleResolution={resolution}]"
            scenarios.append({"name": label, "cwd": cwd, "caseSensitive": case_sensitive, "files": json_files, "options": o, "requests": requests})
    return scenarios


def main():
    count = 0
    for suite in ("compiler", "conformance"):
        for dirpath, _, filenames in os.walk(os.path.join(ROOT, suite)):
            for fn in sorted(filenames):
                if not re.search(r"\.(ts|tsx|js)$", fn):
                    continue
                path = os.path.join(dirpath, fn)
                with open(path, encoding="utf-8", errors="replace") as f:
                    if not INTERESTING.search(f.read()):
                        continue
                for scenario in parse_test(path):
                    sys.stdout.write(json.dumps(scenario) + "\n")
                    count += 1
    sys.stderr.write(f"{count} scenarios\n")


if __name__ == "__main__":
    main()
