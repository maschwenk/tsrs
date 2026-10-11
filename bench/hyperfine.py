#!/usr/bin/env python3
"""Measure the README projects with Hyperfine 2.0, using matching TypeScript semantics.

Projects and the pinned reference compiler must already be set up by bench/run.py --setup-only.
bench/hyperfine.sh runs this on the existing perf-probe workflow's clean release build.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import re
import shlex
import subprocess
from pathlib import Path

import run as bench

PROJECTS = ("vscode", "t3code-server", "mui-docs")
MODES = {"default": [], "checkers4": ["--checkers", "4"]}
FLAGS = ["--noEmit", "--incremental", "false", "--extendedDiagnostics", "--pretty", "false"]
WARMUPS, RUNS = 3, 20
METRICS = {"time_wall_clock": "second", "memory_peak_resident": "byte"}


def output(argv: list[str], cwd: Path | None = None) -> str:
    return subprocess.run(argv, cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n")


def validate_log(path: Path, expected: dict) -> None:
    """Hyperfine keeps the final iteration's output; compare it to the untimed preflight."""
    keys, files, finished = [], None, False
    for line in path.read_text(errors="replace").splitlines():
        if match := bench.ERROR_RE.match(line):
            keys.append(match.group(1))
        elif match := bench.COUNTER_RE.match(line):
            if match.group(1) == "Files":
                files = int(match.group(2))
        elif match := bench.TIME_RE.match(line):
            finished |= match.group(1) == "Check"
    if not finished or files != expected["files"] or sorted(keys) != expected["error_keys"]:
        raise RuntimeError(f"{path}: incomplete check or diagnostics/file count changed after preflight")


def validate_measurements(raw: dict, commands: dict, preflight: dict) -> dict:
    if raw.get("schema_version") != 2 or len(raw.get("results", [])) != len(commands):
        raise RuntimeError("Expected Hyperfine 2.0 JSON with one result per compiler")
    summaries = {}
    for (compiler, command), result in zip(commands.items(), raw["results"]):
        if result["command"] != shlex.join(command):
            raise RuntimeError(f"{compiler}: measured command differs from the recorded command")
        samples = result["measurements"]
        if len(samples) != RUNS or any(s["exit_code"] != preflight[compiler]["exit"] for s in samples):
            raise RuntimeError(f"{compiler}: missing runs or unexpected exit codes")
        for metric, unit in METRICS.items():
            for sample in samples:
                value = sample.get(metric, {})
                if (value.get("unit") != unit or not math.isfinite(value.get("value", math.nan))
                        or value["value"] <= 0):
                    raise RuntimeError(f"{compiler}: invalid {metric} measurement")
        summaries[compiler] = {metric: result["summary"][metric] for metric in METRICS}
    return summaries


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--hyperfine", required=True, type=Path)
    parser.add_argument("--tsrs", required=True, type=Path)
    parser.add_argument("--work", required=True, type=Path)
    parser.add_argument("--out-dir", required=True, type=Path)
    args = parser.parse_args()
    hyperfine, tsrs, work, out = (p.resolve() for p in (args.hyperfine, args.tsrs, args.work, args.out_dir))
    out.mkdir(parents=True, exist_ok=True)
    cfg = json.loads((bench.BENCH / "projects.json").read_text())
    reference = cfg["reference"]
    pinned = re.search(r'^commit = "([0-9a-f]+)"', (bench.REPO / "Cargo.toml").read_text(), re.M)
    if not pinned or pinned.group(1) != reference["commit"]:
        raise RuntimeError("The reference compiler must match the TypeScript commit tsrs ports")
    tsgo = bench.ensure_tsgo(reference, work).resolve()
    compilers = {"typescript": tsgo, "tsrs": tsrs}
    if output([str(hyperfine), "--version"]) != "hyperfine 2.0.0":
        raise RuntimeError("Hyperfine 2.0.0 is required")
    bench.MODE_FLAGS.update(MODES)
    metadata = {
        "started_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "git_commit": output(["git", "rev-parse", "HEAD"], bench.REPO),
        "git_dirty": bool(output(["git", "status", "--porcelain", "--untracked-files=no"], bench.REPO)),
        "machine": bench.machine_info(False, None),
        "build": "cargo build --release --locked -p tsrs_cli",
        "rustc": bench.rustc_version(),
        "typescript_reference": reference,
        "hyperfine": {"version": "2.0.0", "sha256": sha256(hyperfine)},
        "compilers": {name: {"path": str(exe), "version": output([str(exe), "--version"]),
                             "sha256": sha256(exe)} for name, exe in compilers.items()},
        "warmups": WARMUPS, "runs": RUNS, "flags": FLAGS, "modes": MODES,
        "ordering": "sequential compiler batches; compiler order alternates between project/mode pairs",
        "validation": "untimed preflight and final output; every measured exit code must match preflight",
        "projects": {},
    }
    write_json(out / "metadata.json", metadata)

    # Validate every project/mode before starting any timed batches. A valid nonzero exit is a completed check
    # with the same diagnostic positions/codes in both compilers, never a crash or unsupported option.
    for name in PROJECTS:
        project = next(p for p in cfg["projects"] if p["name"] == name)
        cwd, proj = bench.project_path(cfg, project, work)
        commit = output(["git", "rev-parse", "HEAD"], cwd)
        if commit != project["commit"]:
            raise RuntimeError(f"{name}: project checkout does not match bench/projects.json")
        row = {"repo": project["repo"], "commit": commit, "project": project["project"],
               "cwd": str(cwd), "overlay_digest": bench.overlay_digest(project), "modes": {}}
        metadata["projects"][name] = row
        for mode, flags in MODES.items():
            cell = {"commands": {}, "preflight": {}}
            row["modes"][mode] = cell
            for compiler, exe in compilers.items():
                cell["commands"][compiler] = [str(exe), "-p", str(proj), *FLAGS, *flags]
                result = bench.run_once(exe, cwd, proj, mode, out / f"{name}-{mode}-{compiler}-preflight.log", 600)
                cell["preflight"][compiler] = result
                write_json(out / "metadata.json", metadata)
                if not result["ok"] or result.get("files", 0) == 0:
                    raise RuntimeError(f"{name}/{mode}/{compiler}: preflight did not complete")
                if bool(result["exit"]) != bool(result["errors"]):
                    raise RuntimeError(f"{name}/{mode}/{compiler}: inconsistent diagnostics and exit code")
            cell["diagnostics_equal"] = (cell["preflight"]["typescript"]["error_keys"]
                                         == cell["preflight"]["tsrs"]["error_keys"])
            if not cell["diagnostics_equal"]:
                write_json(out / "metadata.json", metadata)
                raise RuntimeError(f"{name}/{mode}: TypeScript and tsrs diagnostics differ")

    batch = 0
    for name, row in metadata["projects"].items():
        for mode, cell in row["modes"].items():
            order = list(compilers) if batch % 2 == 0 else list(reversed(compilers))
            batch += 1
            commands = {compiler: cell["commands"][compiler] for compiler in order}
            raw_path = out / f"{name}-{mode}.json"
            argv = [str(hyperfine), "-N", "--warmup", str(WARMUPS), "--runs", str(RUNS),
                    "--metrics", ",".join(METRICS), "--style", "basic", "--export-json", str(raw_path)]
            exits = sorted({r["exit"] for r in cell["preflight"].values()} - {0})
            if exits:
                argv.append("--ignore-failure=" + ",".join(map(str, exits)))
            for compiler, command in commands.items():
                argv += ["--command-name", compiler, "--output", str(out / f"{name}-{mode}-{compiler}-final.log"),
                         shlex.join(command)]
            cell["hyperfine_command"] = argv
            cell["raw_results"] = raw_path.name
            write_json(out / "metadata.json", metadata)
            bench.log(f"{name}/{mode}: {shlex.join(argv)}")
            subprocess.run(argv, cwd=row["cwd"], check=True, timeout=3600)
            cell["summary"] = validate_measurements(json.loads(raw_path.read_text()), commands, cell["preflight"])
            for compiler in compilers:
                validate_log(out / f"{name}-{mode}-{compiler}-final.log", cell["preflight"][compiler])
            cell["validated"] = True
            write_json(out / "metadata.json", metadata)
    metadata["completed_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
    write_json(out / "metadata.json", metadata)


if __name__ == "__main__":
    main()
