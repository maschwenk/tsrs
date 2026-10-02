#!/usr/bin/env python3
"""LSP oracle: drive two language servers with the same session and diff their responses.

Both servers (by default `$TSRS_WORK/bin/tsgo-ref --lsp -stdio` and `target/release/tsrs --lsp -stdio`) get an
identical, fully sequential session: initialize / initialized, didOpen of every file, then per file the requests
selected with --requests (pull diagnostics, hover and definition at identifier positions, document symbols, ...),
optionally an edit round (incremental didChange, the same requests again, revert), then shutdown / exit.
Server->client requests are answered identically, `window/logMessage` and progress notifications are dropped,
`textDocument/publishDiagnostics` is compared as the last notification per URI.

Inputs:
  --project DIR [--files GLOB...]   open files of an existing project (default: every .ts/.tsx/.js under DIR
                                    except node_modules, capped by --max-files)
  --conformance NAME...             materialize conformance test cases (ts-ref/tsc/testdata/tests/cases/...)
                                    into a temporary directory: `// @filename` splits files, the other `// @key:
                                    value` directives become tsconfig.json compilerOptions

Output: one line per request kind (requests, equal, different), the first --show differences as unified diffs of
the pretty-printed JSON, exit status 1 if anything differs. --dump DIR writes both transcripts.
"""

import argparse
import difflib
import glob
import json
import os
import re
import select
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
TSRS_WORK = os.environ.get("TSRS_WORK", os.path.abspath(os.path.join(REPO, "..", "..")))

IDENT = re.compile(r"[A-Za-z_$][A-Za-z0-9_$]*")
SKIP_LOG_METHODS = {"window/logMessage", "$/progress", "telemetry/event", "window/showMessage", "$/logTrace"}


def utf16_len(s):
    return sum(2 if ord(c) > 0xFFFF else 1 for c in s)


def offset_to_position(text, offset):
    line = text.count("\n", 0, offset)
    start = text.rfind("\n", 0, offset) + 1
    return {"line": line, "character": utf16_len(text[start:offset])}


def file_uri(path):
    from urllib.parse import quote
    return "file://" + quote(os.path.abspath(path))


def strip_comments_and_strings(text):
    """Blank out comments and string literals so identifier positions are code positions (rough, both servers
    get the same positions either way)."""
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        elif c == "/" and i + 1 < n and text[i + 1] == "*":
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif c in "\"'`":
            j = i + 1
            while j < n and text[j] != c and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            for k in range(i, min(j + 1, n)):
                if out[k] != "\n":
                    out[k] = " "
            i = j + 1
        else:
            i += 1
    return "".join(out)


def identifier_offsets(text, limit):
    code = strip_comments_and_strings(text)
    offs = [m.start() for m in IDENT.finditer(code)]
    if len(offs) > limit:
        step = len(offs) / limit
        offs = [offs[int(i * step)] for i in range(limit)]
    return offs


class Server:
    def __init__(self, name, cmd, cwd, log):
        self.name = name
        self.log = log
        self.proc = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=open(log + ".stderr", "w"))
        self.buf = b""
        self.next_id = 0
        self.transcript = []
        self.pushed = {}

    def send(self, msg):
        body = json.dumps(msg, ensure_ascii=False).encode()
        self.proc.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        self.proc.stdin.flush()
        self.transcript.append(("->", msg))

    def _read_raw(self, timeout):
        deadline = time.time() + timeout
        while True:
            sep = self.buf.find(b"\r\n\r\n")
            if sep >= 0:
                headers = self.buf[:sep].decode()
                length = None
                for h in headers.split("\r\n"):
                    k, _, v = h.partition(":")
                    if k.strip().lower() == "content-length":
                        length = int(v.strip())
                if length is not None and len(self.buf) >= sep + 4 + length:
                    body = self.buf[sep + 4:sep + 4 + length]
                    self.buf = self.buf[sep + 4 + length:]
                    return json.loads(body)
            remaining = deadline - time.time()
            if remaining <= 0:
                return None
            r, _, _ = select.select([self.proc.stdout], [], [], remaining)
            if not r:
                return None
            chunk = os.read(self.proc.stdout.fileno(), 1 << 16)
            if not chunk:
                raise EOFError(f"{self.name}: server closed stdout (exit {self.proc.poll()})")
            self.buf += chunk

    def _handle(self, msg):
        """Handles server-initiated traffic; returns True if `msg` was consumed."""
        method = msg.get("method")
        if method is None:
            return False
        if "id" in msg:
            self.transcript.append(("<-req", msg))
            if method == "workspace/configuration":
                result = [None for _ in msg.get("params", {}).get("items", [])]
            else:
                result = None
            self.send({"jsonrpc": "2.0", "id": msg["id"], "result": result})
            return True
        if method in SKIP_LOG_METHODS:
            return True
        self.transcript.append(("<-", msg))
        if method == "textDocument/publishDiagnostics":
            self.pushed[msg["params"]["uri"]] = msg["params"]
        return True

    def request(self, method, params, timeout):
        self.next_id += 1
        rid = self.next_id
        msg = {"jsonrpc": "2.0", "id": rid, "method": method}
        if params is not None:
            msg["params"] = params
        self.send(msg)
        deadline = time.time() + timeout
        while True:
            m = self._read_raw(max(0.0, deadline - time.time()))
            if m is None:
                return {"error": {"code": "timeout", "message": f"no response to {method} within {timeout}s"}}
            if self._handle(m):
                continue
            if m.get("id") == rid:
                self.transcript.append(("<-", m))
                return m
            self.transcript.append(("<-?", m))

    def notify(self, method, params):
        self.send({"jsonrpc": "2.0", "method": method, "params": params})

    def drain(self, seconds):
        deadline = time.time() + seconds
        while time.time() < deadline:
            m = self._read_raw(max(0.0, deadline - time.time()))
            if m is None:
                break
            if not self._handle(m):
                self.transcript.append(("<-?", m))

    def close(self):
        try:
            self.request("shutdown", None, 30)
            self.notify("exit", None)
            self.proc.wait(timeout=10)
        except Exception:
            self.proc.kill()
        with open(self.log, "w") as f:
            for direction, m in self.transcript:
                f.write(direction + " " + json.dumps(m, ensure_ascii=False) + "\n")


def materialize_conformance(names, out):
    roots = [os.path.join(REPO, "ts-ref/tsc/testdata/tests/cases", d) for d in ("compiler", "conformance")]
    made = []
    for name in names:
        path = name if os.path.exists(name) else None
        if path is None:
            for root in roots:
                hits = glob.glob(os.path.join(root, "**", name if name.endswith(".ts") or name.endswith(".tsx") else name + ".ts"), recursive=True)
                if hits:
                    path = hits[0]
                    break
        if path is None:
            sys.exit(f"conformance case not found: {name}")
        base = os.path.splitext(os.path.basename(path))[0]
        d = os.path.join(out, base)
        os.makedirs(d, exist_ok=True)
        text = open(path, encoding="utf-8").read()
        options, files, current, lines = {}, [], os.path.basename(path), []
        for line in text.split("\n"):
            m = re.match(r"^\s*//\s*@(\w+)\s*:\s*(.*?)\s*$", line)
            if m:
                key, value = m.group(1), m.group(2)
                if key.lower() == "filename":
                    if lines and any(l.strip() for l in lines):
                        files.append((current, "\n".join(lines)))
                    current, lines = value, []
                    continue
                v = value
                if v.lower() in ("true", "false"):
                    v = v.lower() == "true"
                elif re.fullmatch(r"\d+", v):
                    v = int(v)
                elif key.lower() in ("lib", "types", "rootdirs", "typeroots"):
                    v = [x.strip() for x in v.split(",") if x.strip()]
                elif "," in v:
                    v = v.split(",")[0].strip()  # multi-variant option: first variant
                options[key] = v
                continue
            lines.append(line)
        files.append((current, "\n".join(lines)))
        for fname, content in files:
            target = os.path.join(d, fname.lstrip("/"))
            os.makedirs(os.path.dirname(target), exist_ok=True)
            with open(target, "w", encoding="utf-8") as f:
                f.write(content)
        if not any(os.path.basename(f) == "tsconfig.json" for f, _ in files):
            with open(os.path.join(d, "tsconfig.json"), "w") as f:
                json.dump({"compilerOptions": options}, f)
        made.append(d)
    return made


def collect_files(project, patterns, max_files):
    if patterns:
        files = []
        for p in patterns:
            files += glob.glob(os.path.join(project, p), recursive=True)
    else:
        files = []
        for root, dirs, names in os.walk(project):
            dirs[:] = [x for x in dirs if x != "node_modules" and not x.startswith(".")]
            for n in names:
                if re.search(r"\.(ts|tsx|mts|cts|js|jsx)$", n):
                    files.append(os.path.join(root, n))
    files = sorted(set(os.path.abspath(f) for f in files))
    return files[:max_files]


def language_id(path):
    if path.endswith((".tsx",)):
        return "typescriptreact"
    if path.endswith((".jsx",)):
        return "javascriptreact"
    if path.endswith((".js", ".mjs", ".cjs")):
        return "javascript"
    return "typescript"


def run_session(server, root, files, args):
    """Runs the scripted session; returns [(key, response)] in request order."""
    results = []
    caps = {
        "textDocument": {
            "synchronization": {"didSave": True},
            "diagnostic": {"dynamicRegistration": False},
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "definition": {"linkSupport": True},
            "documentSymbol": {"hierarchicalDocumentSymbolSupport": True},
            "publishDiagnostics": {"relatedInformation": True, "versionSupport": True},
        },
        "workspace": {"configuration": True, "workspaceFolders": True},
        "general": {"positionEncodings": ["utf-16"]},
    }
    init = server.request("initialize", {
        "processId": None,
        "rootUri": file_uri(root),
        "workspaceFolders": [{"uri": file_uri(root), "name": os.path.basename(root)}],
        "capabilities": caps,
    }, args.timeout)
    results.append(("initialize", init))
    server.notify("initialized", {})
    texts = {}
    for f in files:
        texts[f] = open(f, encoding="utf-8", errors="replace").read()
        server.notify("textDocument/didOpen", {"textDocument": {
            "uri": file_uri(f), "languageId": language_id(f), "version": 1, "text": texts[f]}})

    def per_file_requests(tag):
        for f in files:
            uri = file_uri(f)
            rel = os.path.relpath(f, root)
            text = texts[f]
            if "diagnostic" in args.requests:
                results.append((f"{tag}diagnostic {rel}", server.request(
                    "textDocument/diagnostic", {"textDocument": {"uri": uri}}, args.timeout)))
            if "documentSymbol" in args.requests:
                results.append((f"{tag}documentSymbol {rel}", server.request(
                    "textDocument/documentSymbol", {"textDocument": {"uri": uri}}, args.timeout)))
            offs = identifier_offsets(text, args.positions)
            for method in ("hover", "definition", "typeDefinition", "references", "signatureHelp", "completion"):
                if method not in args.requests:
                    continue
                for off in offs:
                    pos = offset_to_position(text, off)
                    params = {"textDocument": {"uri": uri}, "position": pos}
                    if method == "references":
                        params["context"] = {"includeDeclaration": True}
                    results.append((f"{tag}{method} {rel}:{pos['line'] + 1}:{pos['character'] + 1}",
                                    server.request("textDocument/" + method, params, args.timeout)))

    per_file_requests("")
    if args.edits:
        for f in files[: args.edit_files]:
            uri = file_uri(f)
            text = texts[f]
            offs = identifier_offsets(text, 3)
            if not offs:
                continue
            off = offs[len(offs) // 2]
            pos = offset_to_position(text, off)
            # Insert a character inside an identifier (usually breaks a reference), then revert.
            server.notify("textDocument/didChange", {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [
                {"range": {"start": pos, "end": pos}, "text": "x"}]})
            texts[f] = text[:off] + "x" + text[off:]
            per_file_requests(f"edit({os.path.relpath(f, root)}) ")
            end = {"line": pos["line"], "character": pos["character"] + 1}
            server.notify("textDocument/didChange", {"textDocument": {"uri": uri, "version": 3}, "contentChanges": [
                {"range": {"start": pos, "end": end}, "text": ""}]})
            texts[f] = text
        per_file_requests("reverted ")
    for f in files:
        server.notify("textDocument/didClose", {"textDocument": {"uri": file_uri(f)}})
    server.drain(args.drain)
    for uri in sorted(server.pushed):
        results.append((f"publishDiagnostics {uri}", server.pushed[uri]))
    return results


def normalize(key, msg):
    if msg is None:
        return None
    msg = dict(msg)
    msg.pop("id", None)
    msg.pop("jsonrpc", None)
    if key == "initialize":
        # serverInfo carries the binary's name/version.
        res = msg.get("result")
        if isinstance(res, dict):
            res = dict(res)
            res.pop("serverInfo", None)
            msg["result"] = res
    if key.startswith("completion ") or " completion " in key:
        # Go builds completion lists by iterating Go maps (symbol tables, ...), so its item order differs from run to
        # run (tsgo-ref against itself differs); clients sort by sortText. Compare the items as a multiset.
        res = msg.get("result")
        if isinstance(res, dict) and isinstance(res.get("items"), list):
            res = dict(res)
            res["items"] = sorted(res["items"], key=lambda it: json.dumps(it, sort_keys=True))
            msg["result"] = res
    if "error" in msg and isinstance(msg["error"], dict):
        # Compare error codes, not Go/Rust error text.
        msg["error"] = {"code": msg["error"].get("code")}
    return msg


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--a", default=os.path.join(TSRS_WORK, "bin/tsgo-ref") + " --lsp -stdio")
    ap.add_argument("--b", default=os.path.join(REPO, "target/release/tsrs") + " --lsp -stdio")
    ap.add_argument("--project")
    ap.add_argument("--files", nargs="*")
    ap.add_argument("--conformance", nargs="*")
    ap.add_argument("--max-files", type=int, default=20)
    ap.add_argument("--requests", default="diagnostic,hover,definition")
    ap.add_argument("--positions", type=int, default=40, help="identifier positions per file and method")
    ap.add_argument("--edits", action="store_true")
    ap.add_argument("--edit-files", type=int, default=2)
    ap.add_argument("--timeout", type=float, default=120)
    ap.add_argument("--drain", type=float, default=1.0)
    ap.add_argument("--show", type=int, default=5)
    ap.add_argument("--dump")
    args = ap.parse_args()
    args.requests = set(args.requests.split(","))

    sessions = []
    tmp = None
    if args.conformance:
        tmp = tempfile.mkdtemp(prefix="lsp-oracle-")
        for d in materialize_conformance(args.conformance, tmp):
            sessions.append((d, collect_files(d, None, args.max_files)))
    if args.project:
        root = os.path.abspath(args.project)
        sessions.append((root, collect_files(root, args.files, args.max_files)))
    if not sessions:
        ap.error("need --project or --conformance")

    dump = args.dump or tempfile.mkdtemp(prefix="lsp-oracle-dump-")
    os.makedirs(dump, exist_ok=True)
    stats = {}
    shown = 0
    any_diff = False
    for root, files in sessions:
        name = os.path.basename(root)
        out = {}
        for tag, cmd in (("a", args.a), ("b", args.b)):
            s = Server(tag, cmd.split(), root, os.path.join(dump, f"{name}.{tag}.log"))
            try:
                out[tag] = run_session(s, root, files, args)
            except EOFError as e:
                out[tag] = [("crash", {"error": {"code": "crash", "message": str(e)}})]
            finally:
                s.close()
        a = dict(out["a"])
        b = dict(out["b"])
        keys = [k for k, _ in out["a"]] + [k for k, _ in out["b"] if k not in a]
        for key in keys:
            kind = key.split(" ")[0]
            if kind.startswith("edit(") or kind == "reverted":
                kind = key.split(" ")[1]
            st = stats.setdefault(kind, [0, 0])
            st[0] += 1
            na, nb = normalize(key, a.get(key)), normalize(key, b.get(key))
            if na == nb:
                st[1] += 1
                continue
            any_diff = True
            if shown < args.show:
                shown += 1
                ja = json.dumps(na, indent=1, sort_keys=True, ensure_ascii=False).split("\n")
                jb = json.dumps(nb, indent=1, sort_keys=True, ensure_ascii=False).split("\n")
                print(f"--- {name}: {key}")
                for line in list(difflib.unified_diff(ja, jb, "a", "b", lineterm="", n=2))[2:60]:
                    print(line)
    print(f"{'kind':<22} {'requests':>9} {'equal':>9} {'differ':>9}")
    for kind, (n, eq) in sorted(stats.items()):
        print(f"{kind:<22} {n:>9} {eq:>9} {n - eq:>9}")
    print(f"transcripts: {dump}")
    if tmp:
        shutil.rmtree(tmp, ignore_errors=True)
    sys.exit(1 if any_diff else 0)


if __name__ == "__main__":
    main()
