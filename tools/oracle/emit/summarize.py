#!/usr/bin/env python3
"""Fail-closed report for tools/oracle/emit/monorepo.sh.

    summarize.py <selected names file> <results jsonl>

Prints one line per selected package and a totals line. Exit status 0 only when every selected package has exactly
one well-formed result row and that row is identical: no different/missing/extra file, equal exit statuses that
are tsc ExitStatus values 0-4 (a nonzero status is fine when both compilers agree, e.g. normal diagnostics; 5
NotImplemented, signals/negative codes and other values fail even when equal), equal diagnostics, no tsrs panic and
no timeout. Exit 1 on any mismatch, missing/duplicate/unexpected/malformed row, or an empty selection.

A well-formed row is a JSON object with: name (str), identical/different/missing/extra (int >= 0, not bool),
diagnostics_match (bool), rs_panic (str), ref_status/rs_status (int, not bool, or the string "timeout").
"""

import json
import sys

FIELDS = {"name": str, "identical": int, "different": int, "missing": int, "extra": int, "diagnostics_match": bool, "rs_panic": str}
COUNTS = ("identical", "different", "missing", "extra")
# execute/tsc/compile.go: Success, DiagnosticsPresent_OutputsSkipped/Generated, InvalidProject_OutputsSkipped,
# ProjectReferenceCycle_OutputsSkipped. NotImplemented (5), signals (negative returncodes) and anything else fail.
NORMAL_STATUSES = (0, 1, 2, 3, 4)


def is_int(v):
    return isinstance(v, int) and not isinstance(v, bool)


def row_errors(r):
    if not isinstance(r, dict):
        return [f"not a JSON object ({type(r).__name__})"]
    errors = [k for k, t in FIELDS.items() if not isinstance(r.get(k), t) or (t is int and not is_int(r.get(k)))]
    errors += [f"{k}<0" for k in COUNTS if is_int(r.get(k)) and r[k] < 0]
    for k in ("ref_status", "rs_status"):
        if not (is_int(r.get(k)) or r.get(k) == "timeout"):
            errors.append(f"{k}={r.get(k)!r}")
    return errors


def main():
    selected_path, results_path = sys.argv[1], sys.argv[2]
    selected = [l.strip() for l in open(selected_path) if l.strip()]
    problems = []
    if not selected:
        problems.append("no package selected")
    if len(set(selected)) != len(selected):
        problems.append("duplicate names in the selection")

    rows = {}
    for n, line in enumerate(open(results_path), 1):
        if not line.strip():
            continue
        try:
            r = json.loads(line)
            bad = row_errors(r)
            if bad:
                raise ValueError(f"bad or missing fields {bad}")
        except ValueError as e:
            problems.append(f"malformed result line {n}: {e}: {line.strip()[:120]}")
            continue
        if r["name"] in rows:
            problems.append(f"duplicate result for {r['name']}")
        elif r["name"] not in selected:
            problems.append(f"unexpected result for {r['name']}")
        else:
            rows[r["name"]] = r

    tot = {"identical": 0, "different": 0, "missing": 0, "extra": 0}
    failed = 0
    for name in sorted(selected):
        r = rows.get(name)
        if r is None:
            problems.append(f"no result for {name} (run.py died or produced no JSON)")
            print(f"{name:<45} NO RESULT")
            failed += 1
            continue
        for k in tot:
            tot[k] += r[k]
        reasons = []
        if r["different"] or r["missing"] or r["extra"]:
            reasons.append("files")
        if r["ref_status"] != r["rs_status"]:
            reasons.append("exit status")
        if "timeout" in (r["ref_status"], r["rs_status"]):
            reasons.append("timeout")
        elif any(st not in NORMAL_STATUSES for st in (r["ref_status"], r["rs_status"])):
            reasons.append("abnormal exit")
        if not r["diagnostics_match"]:
            reasons.append("diagnostics")
        if r["rs_panic"]:
            reasons.append("tsrs panic")
        failed += bool(reasons)
        note = f"  [{r['rs_panic'][:90]}]" if r["rs_panic"] else ""
        verdict = "ok" if not reasons else "FAIL(" + ",".join(reasons) + ")"
        print(
            f"{name:<45} {verdict:<14} identical {r['identical']:>5}  different {r['different']:>4}  not-emitted {r['missing']:>5}"
            f"  extra {r['extra']:>3}  exit {r['ref_status']}/{r['rs_status']}  diags {'=' if r['diagnostics_match'] else '!='}{note}"
        )
    print(
        f"packages: {len(selected)} selected, {len(selected) - failed} identical, {failed} failed; files: {tot['identical']} identical,"
        f" {tot['different']} different, {tot['missing']} not emitted by tsrs, {tot['extra']} extra"
    )
    for p in problems:
        print(f"ERROR: {p}")
    ok = not problems and failed == 0
    print("RESULT: " + ("PASS" if ok else "FAIL"))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
