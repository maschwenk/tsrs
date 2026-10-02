#!/usr/bin/env python3
"""Memory of a long-lived language server under edits (docs/LSP.md "Memory plan for a long-lived server").

Starts a server (`--server tsrs` = target/release/tsrs, `--server tsgo` = $TSRS_WORK/bin/tsgo-ref, or `--cmd`),
opens one file of a project, waits for its diagnostics, then sends N incremental edits inside a function body (an
editor typing a statement character by character before a `return`, then deleting it again, repeated). Each edit is
followed by `textDocument/diagnostic` and a hover. The resident set size of the server process (`ps -o rss`) is
sampled every --every edits. Prints one line per sample and a summary; --json writes the samples.

The server is never asked to write anything (no emit, no tsbuildinfo); still, run `git status --short` in a
project you must not modify before and after.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(REPO, "tools", "oracle", "lsp"))
from lsp_oracle import Server, file_uri, language_id, offset_to_position, TSRS_WORK  # noqa: E402

TYPED = "let zz = 1; "


def rss_kib(pid):
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return int(out) if out else 0


def edit_point(text):
    m = re.search(r"^([ \t]+)return\b", text, re.M)
    if not m:
        sys.exit("no `return` statement inside a function body found")
    return m.start() + len(m.group(1))


def hover_point(text, before):
    ids = [m.start() for m in re.finditer(r"\b[A-Za-z_]\w{3,}\b", text[:before])]
    return ids[len(ids) // 2] if ids else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--server", choices=["tsrs", "tsgo"], default="tsrs")
    ap.add_argument("--cmd", help="server command line (overrides --server)")
    ap.add_argument("--project", required=True)
    ap.add_argument("--file", required=True, help="file to open and edit (relative to --project)")
    ap.add_argument("--edits", type=int, default=200)
    ap.add_argument("--every", type=int, default=10)
    ap.add_argument("--timeout", type=float, default=900)
    ap.add_argument("--settle", type=float, default=0.0, help="seconds to wait before the final sample")
    ap.add_argument("--log", default=os.path.join(REPO, "target", "scratch", "mem", "lsp-mem"))
    ap.add_argument("--json")
    ap.add_argument("--env", action="append", default=[], help="KEY=VALUE for the server environment")
    args = ap.parse_args()

    cmd = args.cmd or {
        "tsrs": os.path.join(REPO, "target", "release", "tsrs") + " --lsp -stdio",
        "tsgo": os.path.join(TSRS_WORK, "bin", "tsgo-ref") + " --lsp -stdio",
    }[args.server]
    root = os.path.abspath(args.project)
    path = os.path.join(root, args.file)
    text = open(path, encoding="utf-8").read()
    uri = file_uri(path)
    at = edit_point(text)
    hov = hover_point(text, at)
    os.makedirs(os.path.dirname(args.log), exist_ok=True)
    for kv in args.env:
        k, _, v = kv.partition("=")
        os.environ[k] = v

    server = Server(args.server, cmd.split(), root, args.log)
    pid = server.proc.pid
    samples = []

    def sample(n, label=""):
        r = rss_kib(pid)
        samples.append({"edits": n, "rss_kib": r, "t": round(time.time() - t0, 2)})
        print(f"{label}edits {n:4d}  rss {r / 1024:8.1f} MiB  t {time.time() - t0:7.1f} s", flush=True)

    t0 = time.time()
    server.request("initialize", {
        "processId": None,
        "rootUri": file_uri(root),
        "workspaceFolders": [{"uri": file_uri(root), "name": os.path.basename(root)}],
        "capabilities": {
            "textDocument": {"diagnostic": {"dynamicRegistration": False},
                             "hover": {"contentFormat": ["markdown", "plaintext"]}},
            "workspace": {"configuration": True, "workspaceFolders": True},
            "general": {"positionEncodings": ["utf-16"]},
        },
    }, args.timeout)
    server.notify("initialized", {})
    server.notify("textDocument/didOpen", {"textDocument": {
        "uri": uri, "languageId": language_id(path), "version": 1, "text": text}})
    first = server.request("textDocument/diagnostic", {"textDocument": {"uri": uri}}, args.timeout)
    if "error" in first:
        sys.exit(f"first diagnostic failed: {first['error']}")
    n_items = len((first.get("result") or {}).get("items") or [])
    print(f"opened {args.file}: {n_items} diagnostics, {time.time() - t0:.1f} s", flush=True)
    sample(0)

    cur = text
    typed = 0  # characters of TYPED currently inserted
    direction = 1
    errors = 0
    for i in range(1, args.edits + 1):
        if direction > 0:
            ch = TYPED[typed]
            pos = offset_to_position(cur, at + typed)
            change = {"range": {"start": pos, "end": pos}, "text": ch}
            cur = cur[:at + typed] + ch + cur[at + typed:]
            typed += 1
            if typed == len(TYPED):
                direction = -1
        else:
            start = offset_to_position(cur, at + typed - 1)
            end = offset_to_position(cur, at + typed)
            change = {"range": {"start": start, "end": end}, "text": ""}
            cur = cur[:at + typed - 1] + cur[at + typed:]
            typed -= 1
            if typed == 0:
                direction = 1
        server.notify("textDocument/didChange", {"textDocument": {"uri": uri, "version": i + 1}, "contentChanges": [change]})
        d = server.request("textDocument/diagnostic", {"textDocument": {"uri": uri}}, args.timeout)
        h = server.request("textDocument/hover", {"textDocument": {"uri": uri}, "position": offset_to_position(cur, hov)}, args.timeout)
        errors += ("error" in d) + ("error" in h)
        if i % args.every == 0:
            sample(i)
    if args.settle:
        server.drain(args.settle)
        sample(args.edits, "settled ")
    server.close()
    first_rss, last_rss = samples[0]["rss_kib"], samples[-1]["rss_kib"]
    peak = max(s["rss_kib"] for s in samples)
    print(f"summary {args.server}: start {first_rss / 1024:.1f} MiB, end {last_rss / 1024:.1f} MiB, peak {peak / 1024:.1f} MiB, "
          f"growth {(last_rss - first_rss) / 1024 / max(1, args.edits):.2f} MiB/edit, {errors} errors, {time.time() - t0:.1f} s")
    if args.json:
        with open(args.json, "w") as f:
            json.dump({"server": args.server, "cmd": cmd, "file": args.file, "samples": samples, "errors": errors}, f, indent=1)


if __name__ == "__main__":
    main()
