# Scanner arena string builder experiment

Branch `codex/scanner-arena-builder` starts at `f00d7df0832b90de61d4e792c99bdae181996858`. It adds
`ArenaStringBuilder`, which writes UTF-8 directly into arena storage and grows by allocating a larger block and
copying the initialized bytes. The scanner uses it for concatenation, escaped identifiers and strings, templates,
radix normalization, legacy octal conversion, and numeric-separator assembly.

On five production `ast_oracle bench` rounds over 14,013 public VS Code files (181.7 MB, 22,359,491 nodes),
instructions changed from 13.113-13.114 G to 13.099-13.101 G (about -0.10%), while heap allocations changed from
478,940-478,941 to 398,501-398,502 (-16.8%).

The single-checker `tsrs --noCheck` allocation profile changed heap allocations from 7,709,820 to 7,678,275.
Arena requests changed from 875.2 MB to 875.9 MB because old growth blocks remain in the bump arena. Arena chunk
capacity stayed at 576.0 MB and profiler peak heap stayed at 239.8 MB. Maximum resident set was 1,219,166,208 bytes
against 1,220,427,776 bytes for the base (-0.10%, a noise-sized difference).

Sorted AST-oracle hashes match the base for all 14,013 files. The 108 core tests and six scanner tests pass, as do
workspace compilation, `tools/lint/ratchet.py`, `tools/lint/source.py`, and `git diff --check`. The implementation
is retained in this independent worktree; it is not combined with the numeric-cache experiment.
