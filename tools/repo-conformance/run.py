#!/usr/bin/env python3
"""Compare tsrs and pinned tsgo on popular TypeScript repositories."""

from __future__ import annotations

import argparse
from collections import Counter
import datetime as dt
import difflib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time
from urllib.parse import urlencode
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[2]
QUERY = "language:TypeScript stars:>100 archived:false fork:false"
FILES = re.compile(r"^Files:\s+(\d+)\s*$", re.MULTILINE)
NAME = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\Z")


def write_json(path: Path, data: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n")


def child_env() -> dict[str, str]:
    # Package-manager config can execute code; exclude CI credentials.
    allowed = {"PATH", "HOME", "LANG", "LC_ALL", "TMPDIR", "USER", "LOGNAME"}
    env = {k: v for k, v in os.environ.items() if k in allowed}
    return dict(env, CI="true", GIT_TERMINAL_PROMPT="0", COREPACK_ENABLE_DOWNLOAD_PROMPT="0",
                COREPACK_ENABLE_PROJECT_SPEC="0", npm_config_ignore_scripts="true",
                YARN_ENABLE_SCRIPTS="false", YARN_NODE_LINKER="node-modules", YARN_IGNORE_PATH="1")


def execute(command: list[str], cwd: Path, log: Path, timeout: float) -> dict:
    log.parent.mkdir(parents=True, exist_ok=True)
    start = time.monotonic()
    with log.open("w") as output:
        output.write("$ " + " ".join(command) + "\n")
        output.flush()
        proc = subprocess.Popen(command, cwd=cwd, env=child_env(), stdout=output,
                                stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = proc.wait(timeout=max(0.01, timeout))
            timed_out = False
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            code = proc.wait()
            timed_out = True
    return {"exit": code, "timeout": timed_out, "seconds": round(time.monotonic() - start, 3)}


def discover(count: int, shards: int, out: Path) -> dict:
    headers = {"Accept": "application/vnd.github+json", "User-Agent": "tsrs-repo-conformance"}
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = "Bearer " + token
    repos = {}
    for page in range(1, 11):
        query = urlencode(dict(q=QUERY, sort="stars", order="desc", per_page=100, page=page))
        with urlopen(Request("https://api.github.com/search/repositories?" + query, headers=headers), timeout=60) as response:
            data = json.load(response)
        if data.get("incomplete_results"):
            raise RuntimeError("GitHub returned an incomplete repository search; retry discovery")
        for repo in data["items"]:
            name = repo["full_name"]
            if not NAME.fullmatch(name):
                raise ValueError("Invalid repository name: " + name)
            repos[name] = {"repository": name, "branch": repo["default_branch"], "stars": repo["stargazers_count"]}
        if len(repos) >= count:
            break
        if not data["items"]:
            break
    if len(repos) < count:
        raise RuntimeError(f"Expected {count} repositories, found only {len(repos)}")
    manifest = {
        "created_at": dt.datetime.now(dt.timezone.utc).isoformat(), "query": QUERY,
        "tsrs_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "typescript_commit": subprocess.check_output(["node", "npm/build.mjs", "--print-typescript-commit"], cwd=ROOT, text=True).strip(),
        "shards": min(shards, count), "repositories": list(repos.values())[:count],
    }
    write_json(out, manifest)
    return manifest


def install_command(root: Path) -> list[str] | None:
    package = root / "package.json"
    if not package.exists():
        return None
    manager = json.loads(package.read_text()).get("packageManager", "")
    if manager.startswith("bun@") or (not manager and any((root / p).exists() for p in ("bun.lock", "bun.lockb"))):
        raise ValueError("Bun dependency installation is not supported by this harness")
    if manager.startswith("pnpm@") or (not manager and (root / "pnpm-lock.yaml").exists()):
        version = manager or "pnpm@12.10.1"
        return ["corepack", version, "install", "--frozen-lockfile", "--ignore-scripts"]
    if manager.startswith("yarn@") or (not manager and (root / "yarn.lock").exists()):
        version = manager or ("yarn@4.9.2" if "__metadata:" in (root / "yarn.lock").read_text() else "yarn@1.22.22")
        major = int(version.split("@", 1)[1].split(".", 1)[0])
        flags = ["--immutable", "--mode=skip-build"] if major >= 2 else ["--frozen-lockfile", "--ignore-scripts", "--non-interactive"]
        return ["corepack", version, "install", *flags]
    if manager and not manager.startswith("npm@"):
        raise ValueError("Unsupported package manager: " + manager)
    npm = ["corepack", manager] if manager else ["npm"]
    command = ["ci"] if (root / "package-lock.json").exists() else ["install", "--package-lock=false"]
    return [*npm, *command, "--ignore-scripts", "--no-audit", "--no-fund"]


def read_compilation(result: dict, log: Path, root: Path, binary: Path) -> dict:
    text = log.read_text(errors="replace").partition("\n")[2].replace("\r\n", "\n")
    match = FILES.search(text)
    diagnostic_text = text[:match.start()] if match else text
    diagnostic_text = diagnostic_text.replace(str(root), "<repo>").replace(str(binary.parent), "<compiler>")
    return dict(result, files=int(match[1]) if match else 0, diagnostics=diagnostic_text.rstrip())


def compare(go: dict, rust: dict) -> str:
    if go["timeout"] or go["exit"] not in (0, 1, 2):
        return "reference_failed"
    if rust["timeout"] or rust["exit"] not in (0, 1, 2):
        return "tsrs_failed"
    if any(go[key] != rust[key] for key in ("exit", "files", "diagnostics")):
        return "mismatch"
    if not go["files"]:
        return "no_inputs"
    return "match"


def check_config(root: Path, config: str, tsgo: Path, tsrs: Path, out: Path, timeout: float) -> dict:
    out = out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    compilers = {}
    for label, binary in (("tsgo", tsgo), ("tsrs", tsrs)):
        command = [str(binary), "-p", config, "--noEmit", "--pretty", "false", "--extendedDiagnostics",
                   "--incremental", "--tsBuildInfoFile", str(out / (label + ".tsbuildinfo"))]
        (out / (label + ".tsbuildinfo")).unlink(missing_ok=True)
        log = out / (label + ".log")
        result = execute(command, root, log, timeout)
        compilers[label] = read_compilation(result, log, root, binary)
        (out / (label + ".tsbuildinfo")).unlink(missing_ok=True)
    status = compare(compilers["tsgo"], compilers["tsrs"])
    if status == "mismatch":
        def lines(run: dict) -> list[str]:
            return [f"exit: {run['exit']}\n", f"files: {run['files']}\n", *(s + "\n" for s in run["diagnostics"].splitlines())]
        diff = difflib.unified_diff(lines(compilers["tsgo"]), lines(compilers["tsrs"]), fromfile="tsgo", tofile="tsrs")
        (out / "diagnostics.diff").write_text("".join(diff))
    return {"config": config, "status": status,
            **{label: {k: v for k, v in run.items() if k != "diagnostics"} for label, run in compilers.items()}}


def check_repo(repo: dict, tsgo: Path, tsrs: Path, out: Path, setup_timeout: int,
               check_timeout: int, max_configs: int) -> dict:
    result = dict(repo, status="setup_failed", configs=[])
    with tempfile.TemporaryDirectory(prefix="tsrs-repo-") as temp:
        root = Path(temp) / "repo"
        try:
            deadline = time.monotonic() + setup_timeout
            clone = execute(["git", "clone", "--depth", "1", "--single-branch", "--branch", repo["branch"],
                             "https://github.com/" + repo["repository"] + ".git", str(root)], Path(temp), out / "clone.log", min(60, setup_timeout))
            if clone["exit"] or clone["timeout"]:
                result["reason"] = "Clone failed or timed out; see clone.log"
                return result
            result["commit"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
            tracked = subprocess.check_output(["git", "ls-files", "-z"], cwd=root).decode().split("\0")
            configs = sorted((p for p in tracked if Path(p).name == "tsconfig.json" and "node_modules" not in Path(p).parts),
                             key=lambda p: (len(Path(p).parts), p))
            result["available_configs"] = len(configs)
            if not configs:
                result.update(status="no_config", reason="No tracked tsconfig.json")
                return result
            command = install_command(root)
            if command:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    result["reason"] = "Dependency setup budget exhausted"
                    return result
                install = execute(command, root, out / "install.log", remaining)
                if install["exit"] or install["timeout"]:
                    result["reason"] = "Dependency installation failed or timed out; see install.log"
                    return result
            deadline = time.monotonic() + check_timeout
            for index, config in enumerate(configs[:max_configs]):
                remaining = deadline - time.monotonic()
                if remaining < 1:
                    result["configs"].append({"config": config, "status": "budget_exhausted"})
                    break
                result["configs"].append(check_config(root, config, tsgo, tsrs, out / str(index), min(60, remaining / 2)))
            counts = Counter(c["status"] for c in result["configs"])
            if counts["mismatch"] or counts["tsrs_failed"]:
                result["status"] = "mismatch" if counts["mismatch"] else "tsrs_failed"
            elif counts["match"]:
                result["status"] = "match" if counts["match"] == len(configs) else "partial"
            else:
                result["status"] = "not_checked"
            return result
        except (OSError, ValueError, subprocess.CalledProcessError) as error:
            result["reason"] = str(error)
            return result


def run_shard(args: argparse.Namespace, manifest: dict) -> None:
    if not 0 <= args.shard < manifest["shards"]:
        raise ValueError("Shard outside manifest range")
    for binary in (args.tsgo, args.tsrs):
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError(f"Compiler is not executable: {binary}")
    results = []
    path = args.out / f"shard-{args.shard}.json"
    report = {"tsrs_commit": manifest["tsrs_commit"], "typescript_commit": manifest["typescript_commit"],
              "shard": args.shard, "setup_timeout": args.setup_timeout, "check_timeout": args.check_timeout,
              "max_configs": args.max_configs, "results": results}
    write_json(path, report)
    for repo in manifest["repositories"][args.shard::manifest["shards"]]:
        name = repo["repository"]
        result = check_repo(repo, args.tsgo.resolve(), args.tsrs.resolve(), args.out / name.replace("/", "--"),
                            args.setup_timeout, args.check_timeout, args.max_configs)
        results.append(result)
        write_json(path, report)
        print(f"{name}: {result['status']}", flush=True)


def summarize(manifest: dict, directory: Path, out: Path) -> bool:
    expected = {r["repository"] for r in manifest["repositories"]}
    results, shards, errors = {}, set(), []
    for path in sorted(directory.rglob("shard-*.json")):
        report = json.loads(path.read_text())
        shard = report["shard"]
        if shard in shards or not 0 <= shard < manifest["shards"]:
            errors.append(f"Duplicate or unexpected shard {shard}")
        shards.add(shard)
        for key in ("tsrs_commit", "typescript_commit"):
            if report[key] != manifest[key]:
                errors.append(f"{path.name}: {key} does not match the manifest")
        for result in report["results"]:
            name = result["repository"]
            if name in results or name not in expected:
                errors.append(f"Duplicate or unexpected repository {name}")
            results[name] = result
    missing = expected - results.keys()
    errors.extend(f"Missing repository result: {name}" for name in sorted(missing))
    if shards != set(range(manifest["shards"])):
        errors.append("Some shard reports are missing")
    counts = Counter(r["status"] for r in results.values())
    configs = Counter(c["status"] for r in results.values() for c in r["configs"])
    if not configs["match"] and not configs["mismatch"]:
        errors.append("No project completed a diagnostic comparison")
    lines = ["# Top repository conformance", "",
             f"tsrs `{manifest['tsrs_commit']}` vs TypeScript `{manifest['typescript_commit']}`.", "",
             f"{len(results)}/{len(expected)} repository results; {configs['match']} matching projects; "
             f"{configs['mismatch']} diagnostic differences; {configs['tsrs_failed']} tsrs failures.", "",
             "Repository status: " + ", ".join(f"{status}: {n}" for status, n in sorted(counts.items())) + ".", "",
             "Project status: " + ", ".join(f"{status}: {n}" for status, n in sorted(configs.items())) + ".", "",
             "`partial`: some configs matched; others were skipped or exceeded the limit. "
             "Skipped and empty projects are excluded from the pass count.", "",
             "Checks use `--noEmit`. Artifacts contain logs, diagnostic diffs, and repository commits.", "",
             "| Repository | Commit | Status | Matching / available configs | Detail |", "| --- | --- | --- | --- | --- |"]
    for name in sorted(expected):
        r = results.get(name, {"status": "missing", "configs": []})
        matching = sum(c["status"] == "match" for c in r["configs"])
        detail = r.get("reason") or ", ".join(f"{c['config']}: {c['status']}" for c in r["configs"] if c["status"] != "match")
        detail = detail.replace("|", "\\|").replace("\n", " ")[:500]
        lines.append(f"| [{name}](https://github.com/{name}) | `{r.get('commit', '')}` | {r['status']} | "
                     f"{matching} / {r.get('available_configs', 0)} | {detail} |")
    if errors:
        lines.extend(["", "## Incomplete run", "", *("- " + e for e in errors)])
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(lines) + "\n")
    return not errors and not configs["mismatch"] and not configs["tsrs_failed"]


def positive(value: str) -> int:
    n = int(value)
    if n < 1:
        raise argparse.ArgumentTypeError("Must be positive")
    return n


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    plan = commands.add_parser("plan")
    plan.add_argument("--count", type=positive, default=400)
    plan.add_argument("--shards", type=positive, default=20)
    plan.add_argument("--out", type=Path, required=True)
    run = commands.add_parser("run")
    run.add_argument("--manifest", type=Path, required=True)
    run.add_argument("--shard", type=int, default=0)
    run.add_argument("--tsgo", type=Path, required=True)
    run.add_argument("--tsrs", type=Path, required=True)
    run.add_argument("--out", type=Path, required=True)
    run.add_argument("--setup-timeout", type=positive, default=180)
    run.add_argument("--check-timeout", type=positive, default=120)
    run.add_argument("--max-configs", type=positive, default=5)
    report = commands.add_parser("report")
    report.add_argument("--manifest", type=Path, required=True)
    report.add_argument("--results", type=Path, required=True)
    report.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "plan":
        if args.count > 400 or args.shards > 20:
            parser.error("At most 400 repositories and 20 shards")
        manifest = discover(args.count, args.shards, args.out)
        matrix = json.dumps({"shard": list(range(manifest["shards"]))})
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                output.write("matrix=" + matrix + "\n")
        print(matrix)
    elif args.command == "run":
        run_shard(args, json.loads(args.manifest.read_text()))
    else:
        return 0 if summarize(json.loads(args.manifest.read_text()), args.results, args.out) else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
