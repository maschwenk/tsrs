# Comparator controls

Synthetic captures in the proxy.mjs format that pin down compare-responses.mjs behavior.

- `scen.mjs` and `mk.mjs` come from the codec lane's independent audit of parity `4c50d93`
  (counterexamples CE1–CE5). `scen.mjs` is verbatim, sha256 `e8352cb2…f05392`. `mk.mjs` differs only in
  its output directory.
- `scen-extra.mjs` adds the parity lane's controls:
  - counter-suffix renaming must not touch a user property named `__k@1`;
  - a missing candidate process must fail closed;
  - a differing payload over 4 MiB (hash only) must be inconclusive;
  - sync binary AST payloads must be detected under `--drift`;
  - a genuine reorder where the two oracle runs disagree on order must be accepted.
- `run-controls.mjs` runs every case and exits 1 unless each method gets the required status.
- `baseline/controls-at-4c50d93.out` records the unfixed comparator, where 10 of the 18 cases fail.
