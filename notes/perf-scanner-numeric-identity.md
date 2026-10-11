# Scanner numeric identity experiment

Branch `codex/scanner-numeric-identity` starts at
`f00d7df0832b90de61d4e792c99bdae181996858`. The scanner now avoids numeric-cache entries when source and
canonical text are identical, bypasses the hexadecimal caches for canonical source text, and recognizes canonical
decimal integers of at most 15 digits without conversion or temporary formatting.

On five production `ast_oracle bench` rounds over 14,013 public VS Code files (181.7 MB, 22,359,491 nodes),
instructions changed from 13.113-13.114 G to 13.081-13.085 G (about -0.24%), while heap allocations changed from
478,940-478,941 to 414,436-414,437 (-13.5%).

The single-checker `tsrs --noCheck` allocation profile changed heap allocations from 7,709,820 to 7,664,297.
Arena requests stayed at 875.2 MB, arena chunk capacity stayed at 576.0 MB, and profiler peak heap stayed at
239.8 MB. Maximum resident set was 1,221,705,728 bytes against 1,220,427,776 bytes for the base (+0.10%, a
noise-sized difference).

Sorted AST-oracle hashes match the base for all 14,013 files. Scanner tests, workspace compilation,
`tools/lint/ratchet.py`, `tools/lint/source.py`, and `git diff --check` pass. The allocation reduction is retained
in this independent worktree; it is not combined with the arena-builder experiment.
