#!/usr/bin/env python3
# time to first diagnostics of one opened file (and the messages), for the persisted front-end design notes
import os, sys, time, json
REPO = os.path.expanduser("~/Developer/tsrs-work/wt/persisted-fe")
sys.path.insert(0, os.path.join(REPO, "tools", "oracle", "lsp"))
from lsp_oracle import Server, file_uri, language_id
server_kind, root, rel = sys.argv[1], sys.argv[2], sys.argv[3]
cmd = {"tsrs": os.path.join(REPO, "target/release/tsrs") + " --lsp -stdio",
       "tsgo": os.path.expanduser("~/Developer/tsrs-work/bin/tsgo-ref") + " --lsp -stdio"}[server_kind]
path = os.path.join(root, rel); text = open(path).read()
t0 = time.time()
s = Server(server_kind, cmd.split(), root, os.path.join(REPO, "scratch", "lsp-first-" + server_kind))
s.request("initialize", {"processId": None, "rootUri": file_uri(root),
  "workspaceFolders": [{"uri": file_uri(root), "name": "p"}],
  "capabilities": {"textDocument": {"diagnostic": {"dynamicRegistration": False}}, "workspace": {"configuration": True, "workspaceFolders": True}, "general": {"positionEncodings": ["utf-16"]}}}, 900)
t_init = time.time() - t0
s.notify("initialized", {})
s.notify("textDocument/didOpen", {"textDocument": {"uri": file_uri(path), "languageId": language_id(path), "version": 1, "text": text}})
r = s.request("textDocument/diagnostic", {"textDocument": {"uri": file_uri(path)}}, 900)
t_diag = time.time() - t0
items = (r.get("result") or {}).get("items") or []
r2 = s.request("textDocument/hover", {"textDocument": {"uri": file_uri(path)}, "position": {"line": 0, "character": 10}}, 900)
t_hover = time.time() - t0
print(f"{server_kind}: init {t_init:.2f}s first-diagnostics {t_diag:.2f}s hover {t_hover:.2f}s items {len(items)}")
for it in items[:int(os.environ.get('SHOW', '0'))]:
    print("  ", it.get("code"), it["message"][:150].replace("\n", " "))
