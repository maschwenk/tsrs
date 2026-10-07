#!/usr/bin/env python3
"""Probe for leaf-file freeing (notes/mem-leaf-regions-cost.md): interleaved runs of one tsrs binary under several
environment variants, with the kernel-side numbers next to the wall time.

Per run: wall (`os.wait4`), user and system CPU, peak RSS, minor faults, context switches, the deltas of
/proc/vmstat counters that move with page faults, huge pages, compaction and TLB flushes, and the phase times of
`--extendedDiagnostics`. After the timed runs, per variant: `perf stat` (page faults, dTLB misses, context switches,
user and kernel cycles), `strace -f -c` (the syscall mix) and, if asked, `perf record` of one run with the kernel's
top symbols. Everything goes to $PROBE_OUT; summary.md has the tables.

    tools/perf/leafprobe.py --tsrs target/release/tsrs --work bench/.work --out probe-out \
        --projects vscode,formbricks-web --checkers 1,4 --reps 5 \
        --variant off:TSRS_FREE_LEAVES=0 --variant on:TSRS_FREE_LEAVES=1 [--profile vscode:1] [--no-strace]
"""

import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "bench"))
import run as bench  # noqa: E402

VMSTAT = [
    "pgfault", "pgmajfault", "thp_fault_alloc", "thp_fault_fallback", "thp_fault_fallback_charge", "thp_split_pmd",
    "thp_collapse_alloc", "compact_stall", "compact_fail", "compact_success", "compact_migrate_scanned",
    "compact_free_scanned", "compact_isolated", "pgmigrate_success", "allocstall_normal", "allocstall_movable",
    "pgscan_direct", "pgsteal_direct", "nr_tlb_remote_flush", "nr_tlb_remote_flush_received", "nr_tlb_local_flush_all",
    "nr_tlb_local_flush_one", "zone_reclaim_failed", "pgfree", "pgalloc_normal",
]
PHASES = ["Parse time", "Bind time", "Check time", "Total time"]


def vmstat() -> dict:
    out = {}
    try:
        for line in Path("/proc/vmstat").read_text().splitlines():
            k, v = line.split()
            out[k] = int(v)
    except OSError:
        pass
    return out


def project_cmd(cfg: dict, name: str, tsrs: str, work: Path, checkers: int) -> tuple[Path, list[str]]:
    p = next(p for p in cfg["projects"] if p["name"] == name)
    cwd, proj = bench.project_path(cfg, p, work)
    args = [tsrs, "-p", str(proj), "--noEmit", "--incremental", "false", "--pretty", "false", "--extendedDiagnostics"]
    args += ["--singleThreaded"] if checkers == 1 else ["--checkers", str(checkers)]
    return cwd, args


def timed(cwd: Path, args: list[str], env: dict, out: Path) -> dict:
    before = vmstat()
    with open(out, "wb") as f:
        t0 = time.perf_counter()
        proc = subprocess.Popen(args, cwd=cwd, env=env, stdout=f, stderr=subprocess.STDOUT)
        _, status, ru = os.wait4(proc.pid, 0)
        wall = time.perf_counter() - t0
        proc.returncode = os.waitstatus_to_exitcode(status)
    after = vmstat()
    text = out.read_text(errors="replace")
    r = {
        "wall": wall, "user": ru.ru_utime, "sys": ru.ru_stime, "maxrss_kib": ru.ru_maxrss, "minflt": ru.ru_minflt,
        "majflt": ru.ru_majflt, "nvcsw": ru.ru_nvcsw, "nivcsw": ru.ru_nivcsw, "exit": proc.returncode,
        "vmstat": {k: after.get(k, 0) - before.get(k, 0) for k in VMSTAT if k in after},
        "errors": len(re.findall(r"error TS\d+", text)),
        "stats": next((l for l in text.splitlines() if l.startswith("tsrs: leaf files")), ""),
    }
    for ph in PHASES:
        m = re.search(rf"^{ph}:\s+([\d.]+)s", text, re.M)
        if m:
            r[ph] = float(m.group(1))
    return r


def sh(cmd: list[str], **kw) -> str:
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=1800, **kw).stdout
    except (OSError, subprocess.SubprocessError) as e:
        return f"{cmd[0]}: {e}"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tsrs", required=True)
    ap.add_argument("--work", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--projects", default="vscode,formbricks-web")
    ap.add_argument("--checkers", default="1,4")
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--variant", action="append", default=[], help="name:K=V[,K=V...]")
    ap.add_argument("--profile", action="append", default=[], help="project:checkers to perf record per variant")
    ap.add_argument("--no-strace", action="store_true")
    ap.add_argument("--no-perf-stat", action="store_true")
    args = ap.parse_args()
    variants = []
    for v in args.variant or ["off:TSRS_FREE_LEAVES=0", "on:TSRS_FREE_LEAVES=1"]:
        name, _, kvs = v.partition(":")
        env = dict(kv.split("=", 1) for kv in kvs.split(";") if kv)
        variants.append((name, env))
    cfg = json.loads((ROOT / "bench" / "projects.json").read_text())
    out = args.out
    (out / "runs").mkdir(parents=True, exist_ok=True)
    base_env = {k: v for k, v in os.environ.items() if not k.startswith("TSRS_")}
    machine = {
        "nproc": os.cpu_count(), "uname": " ".join(os.uname()),
        "thp": {f: Path(f"/sys/kernel/mm/transparent_hugepage/{f}").read_text().strip()
                for f in ["enabled", "defrag", "khugepaged/defrag"]
                if Path(f"/sys/kernel/mm/transparent_hugepage/{f}").exists()},
        "meminfo": sh(["head", "-5", "/proc/meminfo"]), "version": sh([args.tsrs, "--version"]).strip(),
    }
    (out / "machine.json").write_text(json.dumps(machine, indent=1))
    print(json.dumps(machine, indent=1), flush=True)
    results: list[dict] = []
    projects = args.projects.split(",")
    checkers = [int(c) for c in args.checkers.split(",")]
    for name in projects:
        for k in checkers:
            cwd, cmd = project_cmd(cfg, name, args.tsrs, args.work, k)
            subprocess.run(cmd, cwd=cwd, env=base_env, capture_output=True)  # warm the page cache
            for rep in range(args.reps):
                for vname, venv in variants:
                    o = out / "runs" / f"{name}-c{k}-{vname}-rep{rep}.txt"
                    r = timed(cwd, cmd, dict(base_env, **venv), o)
                    r.update(project=name, checkers=k, variant=vname, rep=rep)
                    results.append(r)
                    print(f"{name} c{k} {vname:>10} rep{rep}: wall {r['wall']:.3f} user {r['user']:.2f} sys {r['sys']:.2f}"
                          f" rss {r['maxrss_kib'] / 1048576:.2f} GiB minflt {r['minflt']} check {r.get('Check time')}"
                          f" thp {r['vmstat'].get('thp_fault_alloc')} compact {r['vmstat'].get('compact_stall')}"
                          f" errors {r['errors']} exit {r['exit']}", flush=True)
    (out / "results.json").write_text(json.dumps(results, indent=1))

    def med(xs):
        return statistics.median(xs) if xs else float("nan")

    lines = [f"# leaf probe\n\n`{machine['version']}`, {machine['nproc']} CPUs, THP {machine['thp']}\n"]
    lines.append("| project | checkers | variant | wall s (median) | vs first | user s | sys s | peak GiB | vs first | minor faults | thp_fault_alloc | compact_stall | tlb_remote_flush | Parse s | Bind s | Check s | errors |")
    lines.append("| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |")
    for name in projects:
        for k in checkers:
            first = None
            for vname, _ in variants:
                rs = [r for r in results if r["project"] == name and r["checkers"] == k and r["variant"] == vname]
                m = {key: med([r[key] for r in rs]) for key in ["wall", "user", "sys", "maxrss_kib", "minflt"]}
                vm = {key: med([r["vmstat"].get(key, 0) for r in rs]) for key in ["thp_fault_alloc", "compact_stall", "nr_tlb_remote_flush"]}
                ph = {key: med([r[key] for r in rs if key in r]) for key in PHASES}
                first = first or m
                lines.append(
                    f"| {name} | {k} | {vname} | {m['wall']:.3f} | {100 * (m['wall'] / first['wall'] - 1):+.1f}% | {m['user']:.2f} | "
                    f"{m['sys']:.2f} | {m['maxrss_kib'] / 1048576:.3f} | {100 * (m['maxrss_kib'] / first['maxrss_kib'] - 1):+.1f}% | "
                    f"{m['minflt']:.0f} | {vm['thp_fault_alloc']:.0f} | {vm['compact_stall']:.0f} | {vm['nr_tlb_remote_flush']:.0f} | "
                    f"{ph['Parse time']:.3f} | {ph['Bind time']:.3f} | {ph['Check time']:.3f} | {sorted({r['errors'] for r in rs})} |")
    stats_lines = sorted({(r["variant"], r["project"], r["checkers"], r["stats"]) for r in results if r["stats"]})
    if stats_lines:
        lines.append("\n```")
        lines += [f"{v} {p} c{k}: {s}" for v, p, k, s in stats_lines]
        lines.append("```")
    (out / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines), flush=True)

    # The packaged `perf` wrapper refuses kernels it has no tools for (Depot's 6.12); the binary itself works.
    import glob
    perf = next(iter(sorted(glob.glob("/usr/lib/linux-tools/*/perf"), reverse=True)), None) or shutil.which("perf")
    extras = []
    for name in projects:
        for k in checkers:
            cwd, cmd = project_cmd(cfg, name, args.tsrs, args.work, k)
            for vname, venv in variants:
                env = dict(base_env, **venv)
                tag = f"{name}-c{k}-{vname}"
                if perf and not args.no_perf_stat:
                    ev = "page-faults,minor-faults,dTLB-load-misses,dTLB-store-misses,context-switches,cpu-migrations,cycles:u,cycles:k,instructions:u,instructions:k"
                    res = subprocess.run([perf, "stat", "-r", "2", "-e", ev, "-x", ",", "-o", str(out / f"perfstat-{tag}.csv"), "--"] + cmd,
                                         cwd=cwd, env=env, capture_output=True)
                    extras.append(f"### perf stat {tag}\n```\n{(out / f'perfstat-{tag}.csv').read_text() if (out / f'perfstat-{tag}.csv').exists() else res.stderr.decode()[-2000:]}\n```")
                if shutil.which("strace") and not args.no_strace:
                    so = out / f"strace-{tag}.txt"
                    subprocess.run(["strace", "-f", "-c", "-o", str(so), "--"] + cmd, cwd=cwd, env=env, capture_output=True)
                    if so.exists():
                        extras.append(f"### strace -f -c {tag}\n```\n{so.read_text()[:4000]}\n```")
    for spec in args.profile:
        name, _, k = spec.partition(":")
        k = int(k or 1)
        if not perf:
            break
        cwd, cmd = project_cmd(cfg, name, args.tsrs, args.work, k)
        for vname, venv in variants:
            env = dict(base_env, **venv)
            tag = f"{name}-c{k}-{vname}"
            data = out / f"perf-{tag}.data"
            subprocess.run([perf, "record", "-F", "999", "-g", "-o", str(data), "--"] + cmd, cwd=cwd, env=env, capture_output=True)
            for sort, what in [("dso", "dso"), ("sym", "sym")]:
                rep = sh([perf, "report", "-i", str(data), "--no-children", "--sort", sort, "--stdio", "-g", "none", "--percent-limit", "0.3"])
                (out / f"perf-{tag}-{what}.txt").write_text(rep)
                extras.append(f"### perf report --sort {sort} {tag}\n```\n{rep[:6000]}\n```")
            # Kernel symbols: every `[k]` line of a flat report (the kernel is a few percent of the samples).
            flat = sh([perf, "report", "-i", str(data), "--no-children", "--sort", "sym", "--stdio", "-g", "none", "--percent-limit", "0.005"])
            kern = "\n".join(l for l in flat.splitlines() if "[k]" in l)
            (out / f"perf-{tag}-kernel-callers.txt").write_text(kern)
            extras.append(f"### kernel symbols with callers {tag}\n```\n{kern[:12000]}\n```")
        datas = [out / f"perf-{name}-c{k}-{vname}.data" for vname, _ in variants]
        for other in datas[1:]:
            if datas[0].exists() and other.exists():
                d = subprocess.run([perf, "diff", "--sort", "dso,sym", str(datas[0]), str(other)], capture_output=True, text=True)
                d = d.stdout + d.stderr[-2000:]
                (out / f"perf-diff-{datas[0].stem}-{other.stem}.txt").write_text(d)
                extras.append(f"### perf diff {datas[0].stem} -> {other.stem}\n```\n{d[:8000]}\n```")
        for data in datas:
            data.unlink(missing_ok=True)
    with open(out / "summary.md", "a") as f:
        f.write("\n\n" + "\n\n".join(extras) + "\n")


if __name__ == "__main__":
    main()
