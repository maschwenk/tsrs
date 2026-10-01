#!/usr/bin/env python3
"""Run tsgo / tsrs on Project one process at a time and append one JSON line per run.

usage: measure.py OUT.jsonl ROUNDS MODE[,MODE] LABEL=BIN[:ENV=V;ENV=V] ...
MODE: single | multi (4 checkers; tsrs gets --checkerAssignment go)
Runs are interleaved per round (label order within a round).
"""
import json, os, re, subprocess, sys, time

PROJECT = "$PRIVATE_PROJECT_ROOT/apps/project"
SCR = os.environ.get("MEASURE_CWD", os.path.dirname(os.path.abspath(__file__)))  # cwd for the runs (nothing is written there)

out, rounds, modes = sys.argv[1], int(sys.argv[2]), sys.argv[3].split(",")
specs = []
for a in sys.argv[4:]:
    label, rest = a.split("=", 1)
    env = {}
    if ":" in rest:
        rest, envs = rest.split(":", 1)
        for kv in envs.split(";"):
            if kv:
                k, v = kv.split("=", 1)
                env[k] = v
    specs.append((label, rest, env))

def parse(stdout, stderr):
    r = {}
    for key in ["Symbols", "Types", "Instantiations", "Memory used", "Memory allocs", "Check time", "Total time", "Errors"]:
        m = re.search(r"^" + key + r":\s+([\d.]+)", stdout, re.M)
        if m:
            r[key] = float(m.group(1))
    m = re.search(r"([\d.]+) real", stderr)
    r["real"] = float(m.group(1)) if m else None
    m = re.search(r"(\d+)\s+maximum resident set size", stderr)
    r["maxrss"] = int(m.group(1)) if m else None
    m = re.search(r"(\d+)\s+peak memory footprint", stderr)
    r["peak"] = int(m.group(1)) if m else None
    r["errors_lines"] = len(re.findall(r"error TS\d+", stdout))
    return r

for rnd in range(rounds):
    for mode in modes:
        for label, binpath, env in specs:
            is_tsrs = os.path.basename(binpath) == "tsrs"
            args = [binpath, "-p", PROJECT, "--noEmit", "--incremental", "false", "--extendedDiagnostics"]
            if mode == "single":
                args.append("--singleThreaded")
            else:
                args += ["--checkers", "4"]
                if is_tsrs:
                    args += ["--checkerAssignment", "go"]
            e = dict(os.environ)
            e.update(env)
            t0 = time.time()
            p = subprocess.run(["/usr/bin/time", "-l"] + args, cwd=SCR, env=e, capture_output=True, text=True)
            r = parse(p.stdout, p.stderr)
            r.update(label=label, mode=mode, round=rnd, exit=p.returncode, load=os.getloadavg()[0], env=env)
            with open(out, "a") as f:
                f.write(json.dumps(r) + "\n")
            print(f"{label:28s} {mode:6s} r{rnd} sym={r.get('Symbols')} types={r.get('Types')} inst={r.get('Instantiations')} "
                  f"check={r.get('Check time')} mem={r.get('Memory used')} peak={r.get('peak')} load={r['load']:.0f} exit={p.returncode}", flush=True)
