#!/usr/bin/env python3
"""Mutation harness: compare tsrs diagnostics against the reference compiler on a mutated
copy of a real project.

  mutate.py batch --seed 1 --count 250        pick files, mutate one site each, run both, diff
  mutate.py rerun <batch-dir> [--only IDS]    re-apply (a subset of) a batch's mutations, run, diff
  mutate.py bisect <batch-dir> --key K        find the mutation(s) needed for a diverging diagnostic
  mutate.py restore                           restore every file a batch may have left mutated
  mutate.py summary                           table over all batches

Environment (defaults fit the fix-project setup):
  MUT_CLONE     project dir inside the disposable clone (files are mutated here)
  MUT_PRISTINE  the same project dir in the read-only original (files are restored from here)
  MUT_OUT       output directory for batches
  MUT_REF / MUT_OURS  compiler binaries
"""

import argparse
import collections
import json
import os
import random
import re
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from mutators import MUTATORS, pick_mutation  # noqa: E402
from tslex import LexError  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
CLONE = os.environ.get("MUT_CLONE", "$TSRS_WORK/project-clone/apps/project")
PRISTINE = os.environ.get(
    "MUT_PRISTINE",
    "$PRIVATE_PROJECT_ROOT/apps/project",
)
OUT = os.environ.get("MUT_OUT", os.path.join(REPO, "target/scratch/fix-project/mutate"))
REF = os.environ.get("MUT_REF", "$TSRS_WORK/bin/tsgo-ref")
OURS = os.environ.get("MUT_OURS", os.path.join(REPO, "target/release/tsrs"))

HEAVY_PATH = re.compile(r"orm/entities|epositor|[Ww]orkflow|[Ss]chema|/router/|[Rr]oute|[Ee]ndpoint|/temporal/|[Cc]ontract|/activities/")
HEAVY_TEXT = re.compile(r"z\.object\(|z\.enum\(|@Entity|@Property|\bem\.|EntityManager|@temporalio|proxyActivities|defineEndpoint|createEndpoint|\.route\(")

DIAG_RE = re.compile(r"^(.*)\((\d+),(\d+)\): error TS(\d+): (.*)$")
GLOBAL_RE = re.compile(r"^error TS(\d+): (.*)$")


def dirty_path():
    return os.path.join(OUT, "DIRTY")


def program_files():
    cache = os.path.join(OUT, "program-files.txt")
    if not os.path.exists(cache):
        os.makedirs(OUT, exist_ok=True)
        r = subprocess.run([REF, "-p", ".", "--listFilesOnly"], cwd=CLONE, capture_output=True, text=True)
        files = []
        prefix = os.path.realpath(CLONE) + "/"
        for line in r.stdout.splitlines():
            p = os.path.realpath(line.strip())
            if p.startswith(prefix) and "/node_modules/" not in p and p.endswith(".ts") and not p.endswith(".d.ts"):
                files.append(p[len(prefix):])
        with open(cache, "w") as f:
            f.write("\n".join(files) + "\n")
    return open(cache).read().split()


def read(p):
    with open(p, encoding="utf-8") as f:
        return f.read()


def restore_file(rel):
    dst = os.path.join(CLONE, rel)
    src = os.path.join(PRISTINE, rel)
    os.remove(dst)
    subprocess.run(["cp", "-c", src, dst], check=True)


def restore_all():
    p = dirty_path()
    if not os.path.exists(p):
        return 0
    rels = [x for x in open(p).read().split("\n") if x]
    for rel in rels:
        restore_file(rel)
    for rel in rels:
        assert read(os.path.join(CLONE, rel)) == read(os.path.join(PRISTINE, rel)), rel
    os.remove(p)
    return len(rels)


def line_col(src, off):
    line = src.count("\n", 0, off) + 1
    col = off - (src.rfind("\n", 0, off) + 1) + 1
    return line, col


def apply_mutations(muts):
    """Writes the mutated files. Each mutation carries the original text of its span, which is
    checked against the pristine file first."""
    by_file = collections.defaultdict(list)
    for m in muts:
        by_file[m["file"]].append(m)
    os.makedirs(OUT, exist_ok=True)
    with open(dirty_path(), "a") as f:
        f.write("\n".join(by_file) + "\n")
    for rel, ms in by_file.items():
        src = read(os.path.join(PRISTINE, rel))
        cur = read(os.path.join(CLONE, rel))
        if cur != src:
            raise SystemExit(f"{rel}: clone differs from pristine before mutation (run `mutate.py restore`)")
        for m in sorted(ms, key=lambda m: -m["start"]):
            assert src[m["start"]:m["end"]] == m["orig"], (rel, m["id"])
            src = src[:m["start"]] + m["repl"] + src[m["end"]:]
        with open(os.path.join(CLONE, rel), "w", encoding="utf-8") as f:
            f.write(src)


def parse_diags(text):
    diags = []
    for line in text.splitlines():
        m = DIAG_RE.match(line)
        if m:
            f = m.group(1)
            if f.startswith("./"):
                f = f[2:]
            diags.append({"file": f, "line": int(m.group(2)), "col": int(m.group(3)), "code": int(m.group(4)),
                          "msg": m.group(5), "more": []})
            continue
        m = GLOBAL_RE.match(line)
        if m:
            diags.append({"file": "", "line": 0, "col": 0, "code": int(m.group(1)), "msg": m.group(2), "more": []})
            continue
        if diags and (line.startswith(" ") or line.startswith("\t")):
            diags[-1]["more"].append(line)
    return diags


def run_compilers(outdir):
    os.makedirs(outdir, exist_ok=True)
    ref_out = open(os.path.join(outdir, "ref.txt"), "w")
    our_out = open(os.path.join(outdir, "ours.txt"), "w")
    t0 = time.time()
    pref = subprocess.Popen([REF, "-p", ".", "--noEmit", "--incremental", "false", "--pretty", "false", "--singleThreaded"],
                            cwd=CLONE, stdout=ref_out, stderr=subprocess.STDOUT)
    pours = subprocess.Popen([OURS, "-p", ".", "--pretty", "false"], cwd=CLONE, stdout=our_out, stderr=subprocess.STDOUT)
    rc_ours = pours.wait()
    t_ours = time.time() - t0
    rc_ref = pref.wait()
    t_ref = time.time() - t0
    ref_out.close()
    our_out.close()
    return {"rc_ref": rc_ref, "rc_ours": rc_ours, "t_ref": round(t_ref, 1), "t_ours": round(t_ours, 1)}


def key(d):
    return (d["file"], d["line"], d["col"], d["code"])


def diff(outdir, muts):
    ref = parse_diags(read(os.path.join(outdir, "ref.txt")))
    ours = parse_diags(read(os.path.join(outdir, "ours.txt")))
    rc = collections.Counter(key(d) for d in ref)
    oc = collections.Counter(key(d) for d in ours)
    missing = rc - oc
    extra = oc - rc
    # message text for matched keys where our text has no placeholder type
    by_key_ref = collections.defaultdict(list)
    for d in ref:
        by_key_ref[key(d)].append(d)
    by_key_ours = collections.defaultdict(list)
    for d in ours:
        by_key_ours[key(d)].append(d)
    text_diffs = []
    placeholder = 0
    for k in by_key_ours:
        for a, b in zip(by_key_ref.get(k, []), by_key_ours[k]):
            ours_full = "\n".join([b["msg"]] + b["more"])
            ref_full = "\n".join([a["msg"]] + a["more"])
            if ours_full == ref_full:
                continue
            if "type#" in ours_full or "'\"/" in ours_full:  # placeholder type / module symbol names
                placeholder += 1
                continue
            text_diffs.append({"key": list(k), "ref": ref_full, "ours": ours_full})
    muts_by_file = collections.defaultdict(list)
    for m in muts:
        muts_by_file[m["file"]].append(m["id"])

    def attrib(k):
        return muts_by_file.get(k[0], [])

    files_with_ref_diags = {d["file"] for d in ref}
    effective = sum(1 for m in muts if m["file"] in files_with_ref_diags)
    res = {
        "ref_count": len(ref),
        "ours_count": len(ours),
        "missing": [{"key": list(k), "n": n, "muts": attrib(k), "msg": by_key_ref[k][0]["msg"]} for k, n in sorted(missing.items())],
        "extra": [{"key": list(k), "n": n, "muts": attrib(k), "msg": by_key_ours[k][0]["msg"]} for k, n in sorted(extra.items())],
        "text_diffs": text_diffs,
        "placeholder_text_diffs": placeholder,
        "mutations": len(muts),
        "files": len({m["file"] for m in muts}),
        "effective_mutations": effective,
        "codes": dict(collections.Counter(d["code"] for d in ref).most_common()),
    }
    with open(os.path.join(outdir, "diff.json"), "w") as f:
        json.dump(res, f, indent=1)
    return res


def print_diff(res, limit=40):
    print(f"ref {res['ref_count']} diags, ours {res['ours_count']}; mutations {res['mutations']} in {res['files']} files, "
          f"{res['effective_mutations']} with ref diagnostics in the mutated file")
    print(f"missing {sum(x['n'] for x in res['missing'])}, extra {sum(x['n'] for x in res['extra'])}, "
          f"text diffs {len(res['text_diffs'])} (+{res['placeholder_text_diffs']} placeholder)")
    for kind in ("missing", "extra"):
        for x in res[kind][:limit]:
            print(f"  {kind:7} {x['key']} x{x['n']} muts={x['muts']} :: {x['msg'][:140]}")
    for x in res["text_diffs"][:limit]:
        print(f"  text    {x['key']}\n     ref:  {x['ref'][:300]}\n     ours: {x['ours'][:300]}")


def choose_files(rng, count, used, heavy_frac=0.45, test_frac=0.15):
    files = [f for f in program_files() if f not in used]
    rng.shuffle(files)
    heavy, tests, other = [], [], []
    want_heavy = int(count * heavy_frac)
    want_test = int(count * test_frac)
    want_other = count - want_heavy - want_test
    for f in files:
        if len(heavy) >= want_heavy and len(tests) >= want_test and len(other) >= want_other:
            break
        if f.endswith(".test.ts") or "/tests/" in f or f.endswith(".spec.ts"):
            if len(tests) < want_test:
                tests.append(f)
            continue
        is_heavy = bool(HEAVY_PATH.search(f))
        if not is_heavy and len(heavy) < want_heavy:
            try:
                is_heavy = bool(HEAVY_TEXT.search(read(os.path.join(PRISTINE, f))))
            except OSError:
                continue
        if is_heavy:
            if len(heavy) < want_heavy:
                heavy.append(f)
            elif len(other) < want_other:
                other.append(f)
        elif len(other) < want_other:
            other.append(f)
    return heavy + tests + other


def one_mutation(seed, rel, attempt, weights):
    src = read(os.path.join(PRISTINE, rel))
    rng = random.Random(f"{seed}:{rel}" + (f":{attempt}" if attempt else ""))
    try:
        e = pick_mutation(rel, src, rng, weights)
    except (LexError, RecursionError, IndexError, AssertionError, KeyError) as ex:
        print(f"  skip {rel}: {type(ex).__name__} {ex}", file=sys.stderr)
        return None
    if e is None:
        return None
    line, col = line_col(src, e.start)
    new = src[:e.start] + e.repl + src[e.end:]
    if new == src:
        return None
    return {"file": rel, "mutator": e.mutator, "start": e.start, "end": e.end,
            "orig": src[e.start:e.end], "repl": e.repl, "line": line, "col": col, "note": e.note}, new


def syntax_errors(texts):
    """Files (of {rel: text}) that have syntactic diagnostics; semantic diagnostics are only
    reported for a program without any, so every mutation must keep its file parseable."""
    d = os.path.join(OUT, "synchk")
    shutil.rmtree(d, ignore_errors=True)
    for rel, text in texts.items():
        p = os.path.join(d, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8") as f:
            f.write(text)
    r = subprocess.run([REF, "--noCheck", "--noLib", "--noResolve", "--noEmit", "--pretty", "false", "--types", "",
                        "--experimentalDecorators", "--module", "preserve", "--target", "esnext"] + list(texts),
                       cwd=d, capture_output=True, text=True)
    bad = set()
    for dg in parse_diags(r.stdout):
        if dg["file"]:
            bad.add(dg["file"])
    return bad


def make_mutations(seed, files, weights=None):
    chosen = {}
    pending = list(files)
    for attempt in range(5):
        texts = {}
        for rel in pending:
            r = one_mutation(seed, rel, attempt, weights)
            if r is not None:
                chosen[rel], texts[rel] = r
        if not texts:
            break
        bad = syntax_errors(texts)
        for rel in bad:
            chosen.pop(rel, None)
        pending = sorted(bad)
        if not pending:
            break
    muts = []
    for rel in files:
        if rel in chosen:
            m = chosen[rel]
            m["id"] = len(muts)
            muts.append(m)
    return muts


def used_files():
    used = set()
    if os.path.isdir(OUT):
        for d in os.listdir(OUT):
            p = os.path.join(OUT, d, "mutations.json")
            if d.startswith("batch-") and os.path.exists(p):
                used.update(m["file"] for m in json.load(open(p)))
    return used


def run_batch_dir(outdir, muts):
    if restore_all():
        print("restored files left mutated by an earlier run")
    try:
        apply_mutations(muts)
        info = run_compilers(outdir)
    finally:
        restore_all()
    res = diff(outdir, muts)
    res.update(info)
    with open(os.path.join(outdir, "diff.json"), "w") as f:
        json.dump(res, f, indent=1)
    return res


def cmd_batch(a):
    rng = random.Random(a.seed)
    files = choose_files(rng, a.count, set() if a.allow_reuse else used_files(), a.heavy, a.tests)
    muts = make_mutations(a.seed, files)
    outdir = os.path.join(OUT, f"batch-{a.seed:03d}")
    os.makedirs(outdir, exist_ok=True)
    with open(os.path.join(outdir, "mutations.json"), "w") as f:
        json.dump(muts, f, indent=1)
    print(f"batch {a.seed}: {len(muts)} mutations in {len({m['file'] for m in muts})} files; "
          + ", ".join(f"{k}={v}" for k, v in collections.Counter(m["mutator"] for m in muts).most_common()))
    res = run_batch_dir(outdir, muts)
    print(f"times: ref {res['t_ref']}s ours {res['t_ours']}s (rc {res['rc_ref']}/{res['rc_ours']})")
    print_diff(res)


def load_muts(bdir, only=None):
    muts = json.load(open(os.path.join(bdir, "mutations.json")))
    if only is not None:
        muts = [m for m in muts if m["id"] in only]
    return muts


def parse_ids(s):
    if s is None:
        return None
    out = set()
    for part in s.split(","):
        if "-" in part:
            a, b = part.split("-")
            out.update(range(int(a), int(b) + 1))
        elif part:
            out.add(int(part))
    return out


def cmd_rerun(a):
    muts = load_muts(a.batch, parse_ids(a.only))
    outdir = a.out or os.path.join(a.batch, "rerun")
    res = run_batch_dir(outdir, muts)
    print_diff(res)


def diverges(res, k):
    k = list(k)
    return any(x["key"] == k for x in res["missing"] + res["extra"])


def cmd_bisect(a):
    """Shrinks the mutation set of a batch while the divergence at --key persists."""
    k = json.loads(a.key)
    muts = load_muts(a.batch)
    cands = muts
    n = 0

    def test(sub):
        nonlocal n
        n += 1
        outdir = os.path.join(a.batch, f"bisect-{n}")
        res = run_batch_dir(outdir, sub)
        d = diverges(res, k)
        print(f"  try {len(sub)} muts -> {'DIVERGES' if d else 'ok'}", flush=True)
        return d

    same = [m for m in cands if m["file"] == k[0]]
    if same and test(same):
        cands = same
    # ddmin-lite: halve while one half reproduces
    while len(cands) > 1:
        half = len(cands) // 2
        a1, a2 = cands[:half], cands[half:]
        if test(a1):
            cands = a1
        elif test(a2):
            cands = a2
        else:
            # need elements from both halves: try removing chunks
            changed = False
            for chunk in (a1, a2):
                if len(chunk) > 1:
                    rest = [m for m in cands if m not in chunk]
                    for i in range(0, len(chunk), max(1, len(chunk) // 4)):
                        trial = rest + chunk[:i] + chunk[i + max(1, len(chunk) // 4):]
                        if len(trial) < len(cands) and test(trial):
                            cands = trial
                            changed = True
                            break
                if changed:
                    break
            if not changed:
                break
    print("minimal:", json.dumps(cands, indent=1))


def cmd_restore(_a):
    print(f"restored {restore_all()} files")


def cmd_summary(_a):
    rows = []
    allfiles = set()
    tot = collections.Counter()
    for d in sorted(os.listdir(OUT)):
        p = os.path.join(OUT, d, "diff.json")
        if not d.startswith("batch-") or not os.path.exists(p):
            continue
        r = json.load(open(p))
        muts = json.load(open(os.path.join(OUT, d, "mutations.json")))
        allfiles.update(m["file"] for m in muts)
        miss = sum(x["n"] for x in r["missing"])
        extra = sum(x["n"] for x in r["extra"])
        rows.append((d, r["mutations"], r["files"], r["effective_mutations"], r["ref_count"], miss, extra, len(r["text_diffs"])))
        for m in muts:
            tot[m["mutator"]] += 1
    print("| batch | sites | files | sites w/ diag | ref diags | missing | extra | text diffs |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for r in rows:
        print("| " + " | ".join(str(x) for x in r) + " |")
    print(f"total sites {sum(r[1] for r in rows)}, distinct files {len(allfiles)}")
    print(", ".join(f"{k}={v}" for k, v in tot.most_common()))


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("batch")
    b.add_argument("--seed", type=int, required=True)
    b.add_argument("--count", type=int, default=250)
    b.add_argument("--heavy", type=float, default=0.45)
    b.add_argument("--tests", type=float, default=0.15)
    b.add_argument("--allow-reuse", action="store_true")
    b.set_defaults(fn=cmd_batch)
    r = sub.add_parser("rerun")
    r.add_argument("batch")
    r.add_argument("--only")
    r.add_argument("--out")
    r.set_defaults(fn=cmd_rerun)
    s = sub.add_parser("bisect")
    s.add_argument("batch")
    s.add_argument("--key", required=True, help='JSON ["file", line, col, code]')
    s.set_defaults(fn=cmd_bisect)
    sub.add_parser("restore").set_defaults(fn=cmd_restore)
    sub.add_parser("summary").set_defaults(fn=cmd_summary)
    g = sub.add_parser("gen", help="print the mutations a batch would make, without running")
    g.add_argument("--seed", type=int, required=True)
    g.add_argument("--count", type=int, default=250)
    g.set_defaults(fn=lambda a: [print(json.dumps(m)) for m in make_mutations(a.seed, choose_files(random.Random(a.seed), a.count, used_files()))])
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
