# lsp-mem: memory of the long-lived server (phase 4, docs/LSP.md "Memory plan for a long-lived server")

Wave agent `mem`, branch `lsp-mem`.

## Step 1: measurements (before any change)

Driver: `tools/lsp-mem/lsp_mem.py` (opens one file, waits for its diagnostics, then N incremental edits inside a
function body: typing `let zz = 1; ` before a `return` character by character and deleting it again; each edit is
followed by `textDocument/diagnostic` and a hover; RSS of the server process sampled with `ps -o rss`).

Base `lsp` 0c4b45a, release build, 200 edits, RSS MiB at edit 0 / 20 / 40 / ... / 200:

| project (file) | server | 0 | 20 | 40 | 60 | 80 | 100 | 120 | 140 | 160 | 180 | 200 | MiB/edit |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| xstate (`packages/core/src/createActor.ts`) | tsrs | 187 | 354 | 521 | 689 | 858 | 1027 | 1196 | 1365 | 1534 | 1704 | 1874 | 8.44 |
| xstate | tsgo-ref | 313 | 691 | 698 | 727 | 737 | 738 | 752 | 753 | 754 | 965 | 965 | 3.26 (GC heap target, plateaus) |
| private monorepo (a 408-line service file) | tsrs | 2750 | 2953 | 3324 | 3707 | 4088 | 4473 | 4858 | 5242 | 5627 | 6012 | 6397 | 18.2 |
| private monorepo | tsgo-ref | 4920 | 5282 | 5725 | 6282 | 6702 | 7123 | 7445 | 7445 | 7446 | 7447 | 7449 | 12.7 (plateau from edit 120) |

tsrs grows linearly (every program version, its checkers' types, the old file versions and the per-version
`Program` heap data are kept forever); tsgo-ref grows until its GC heap target and stays flat. The private
monorepo was checked with `git status --short` before and after every run: unchanged (0 entries).
