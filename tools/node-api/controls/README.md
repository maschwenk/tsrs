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
- `scen2.mjs` comes from the codec lane's re-audit of `c3f1717` (N1–N4), sha256 `d0c3b6e2…c471de`. Only the
  `mk.mjs` import and the work-tree constant of the async-only-binary scenario are adapted to this checkout.
  `baseline/codec-run2-at-c3f1717.out` is that audit reproduced unchanged here.
- `scen3.mjs` comes from the codec lane's re-audit of `0333222` (R1, R2), sha256 `79804ba1…27f5fd`, adapted the same
  way. `baseline/codec-run3-at-0333222.out` is that audit reproduced here; all statuses are identical, and only the
  base64 placeholder length differs because the checkout path differs. `scen-extra.mjs` `n1c` is the mirror of
  R1, where run 1 errored and run 2 answered.
- `scen4.mjs` comes from the codec lane's audit of `385d300` (D1, T2, T3), sha256 `fbe68138…dfdb3`. Only the
  `mk.mjs` import is adapted. `baseline/codec-run4-at-385d300.out` is that audit reproduced here, and it is
  identical. `scen-extra.mjs` `d1-*` adds the present/missing and locked/unlocked mirrors.
