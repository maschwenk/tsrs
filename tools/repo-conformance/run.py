#!/usr/bin/env python3
"""Compare tsrs and pinned tsgo on popular TypeScript repositories."""

from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import dataclass
from concurrent.futures import ThreadPoolExecutor
import datetime as dt
import difflib
import json
import os
from pathlib import Path
import platform
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
DIAGNOSTIC = re.compile(r"^(?:.+\(\d+,\d+\): )?(?:error|warning|message) TS\d+: ")
# Extra flags per comparison mode, recorded in shard reports. Default runs both compilers as users do, with the
# project's own options. Strict pins tsgo to its default 4 checkers (newCheckerPoolWithTracing,
# internal/compiler/checkerpool.go: 1 under singleThreaded, never more than the file count) and has tsrs reproduce
# Go's checker assignment, history and caches; lazy members change which instantiations a checker has cached and so
# where depth limits are hit (docs/DEBUGGING.md).
MODES = {"default": {"tsgo": [], "tsrs": []},
         "strict": {"tsgo": ["--checkers", "4"],
                    "tsrs": ["--checkerAssignment", "go", "--checkers", "4", "--noLazyMembers"]}}
# Shard reports without "modes" predate strict mode and ran the default commands only.
LEGACY_MODES = {"default": MODES["default"]}
COMPARED = ("match", "mismatch", "tsrs_failed")
NAME = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")
WORKFLOW = ".github/workflows/repo-conformance.yml"
MAX_SHARDS = 20
# Recorded in manifests. A rerun's workflow comes from the dispatch commit but runs the manifest's harness, so bump
# this when the workflow's arguments, isolation contract or report merging rules change; other versions are refused.
HARNESS = 4


def write_json(path: Path, data: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n")


def child_env(home: Path, cache: Path) -> dict[str, str]:
    """Environment for package managers and compilers. Package-manager config and hooks can run code, so CI
    credentials are left out and HOME is a fresh directory with empty npm and git config: no host config or token is
    reachable through HOME, XDG paths or the environment. Filesystem and process isolation is provided by the
    per-repository container, not this environment filter."""
    allowed = {"PATH", "LANG", "LC_ALL", "TMPDIR", "USER", "LOGNAME"}
    env = {k: v for k, v in os.environ.items() if k in allowed}
    # pnpm 12 ignores TMPDIR for store locks and requires a private XDG runtime directory.
    runtime = home / ".runtime"
    runtime.mkdir(parents=True, exist_ok=True, mode=0o700)
    for path in (home / ".npmrc", home / ".npmrc-global", home / ".gitconfig"):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.touch()
    return dict(env, HOME=str(home), XDG_RUNTIME_DIR=str(runtime),
                XDG_CONFIG_HOME=str(home / ".config"), XDG_STATE_HOME=str(home / ".state"),
                XDG_CACHE_HOME=str(cache / "xdg"), XDG_DATA_HOME=str(cache / "data"),  # pnpm store, Yarn Berry cache
                npm_config_userconfig=str(home / ".npmrc"), npm_config_globalconfig=str(home / ".npmrc-global"),
                npm_config_cache=str(cache / "npm"), COREPACK_HOME=str(cache / "corepack"),
                BUN_INSTALL_CACHE_DIR=str(cache / "bun"), GIT_CONFIG_GLOBAL=str(home / ".gitconfig"),
                GIT_CONFIG_NOSYSTEM="1", CI="true", GIT_TERMINAL_PROMPT="0", COREPACK_ENABLE_DOWNLOAD_PROMPT="0",
                COREPACK_ENABLE_PROJECT_SPEC="0", npm_config_ignore_scripts="true",
                YARN_ENABLE_SCRIPTS="false", YARN_NODE_LINKER="node-modules", YARN_IGNORE_PATH="1")


@dataclass(frozen=True)
class Context:
    tsgo: Path
    tsrs: Path
    env: dict[str, str]


def execute(command: list[str], cwd: Path, log: Path, timeout: float, env: dict[str, str]) -> dict:
    log.parent.mkdir(parents=True, exist_ok=True)
    start = time.monotonic()
    with log.open("w") as output:
        output.write("$ " + " ".join(command) + "\n")
        output.flush()
        proc = subprocess.Popen(command, cwd=cwd, env=env, stdout=output,
                                stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = proc.wait(timeout=max(0.01, timeout))
            timed_out = False
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            code = proc.wait()
            timed_out = True
    return {"exit": code, "timeout": timed_out, "seconds": round(time.monotonic() - start, 3)}


def head_commit(repo: dict, env: dict[str, str]) -> str:
    """The branch's current commit, so every shard and any rerun checks the same snapshot."""
    output = subprocess.run(["git", "ls-remote", "https://github.com/" + repo["repository"] + ".git",
                             "refs/heads/" + repo["branch"]], env=env, capture_output=True, text=True,
                            timeout=60, check=True).stdout.split()
    if not output or not re.fullmatch(r"[0-9a-f]{40}", output[0]):
        raise RuntimeError(f"Could not resolve {repo['repository']} {repo['branch']}")
    return output[0]


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
    selected = list(repos.values())[:count]
    with tempfile.TemporaryDirectory(prefix="tsrs-plan-") as temp, ThreadPoolExecutor(16) as pool:
        env = child_env(Path(temp) / "home", Path(temp) / "cache")
        for repo, commit in zip(selected, pool.map(lambda r: head_commit(r, env), selected)):
            repo["commit"] = commit
    manifest = {
        "created_at": dt.datetime.now(dt.timezone.utc).isoformat(), "query": QUERY,
        "tsrs_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "typescript_commit": subprocess.check_output(["node", "npm/build.mjs", "--print-typescript-commit"], cwd=ROOT, text=True).strip(),
        "harness": HARNESS, "shards": min(shards, count), "repositories": selected,
    }
    write_json(out, manifest)
    return manifest


LOCKFILES = {"bun": ("bun.lock", "bun.lockb"), "pnpm": ("pnpm-lock.yaml",), "yarn": ("yarn.lock",),
             "npm": ("package-lock.json", "npm-shrinkwrap.json")}
# Package managers corepack does not provide, installed from their per-platform npm packages.
TOOLS = {"bun": "@oven/bun-{os}-{arch}", "nub": "@nubjs/nub-{os}-{arch}"}
TOOL_ARCH = {("bun", "arm64"): "aarch64", ("bun", "aarch64"): "aarch64", ("nub", "aarch64"): "arm64",
             ("bun", "x86_64"): "x64", ("nub", "x86_64"): "x64", ("nub", "arm64"): "arm64"}
VERSION = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
                     r"(?:-(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
                     r"(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?\Z")


def check_version(name: str, version: str) -> None:
    base, separator, integrity = version.partition("+")
    valid_hash = name in ("npm", "pnpm", "yarn") and any(
        re.fullmatch(rf"sha{bits}\.[0-9a-fA-F]{{{bits // 4}}}", integrity) for bits in (224, 256, 384, 512))
    if not VERSION.fullmatch(base) or (separator and not valid_hash):
        raise ValueError(f"Invalid package manager version: {name}@{version}")


def declared_manager(package: dict) -> tuple[str, str] | None:
    spec = package.get("packageManager")
    engine = (package.get("devEngines") or {}).get("packageManager")
    if not spec and isinstance(engine, dict) and re.fullmatch(r"\d+\.\d+\.\d+", str(engine.get("version", ""))):
        spec = f"{engine.get('name')}@{engine['version']}"
    if not isinstance(spec, str) or "@" not in spec:
        if spec:
            raise ValueError(f"Invalid packageManager: {spec!r}")
        return None
    name, version = spec.split("@", 1)
    check_version(name, version)
    return name, version


def default_pnpm(root: Path, package: dict) -> str:
    lockfile = root / "pnpm-lock.yaml"
    match = re.match(r"lockfileVersion: '?(\d+)", lockfile.read_text()) if lockfile.exists() else None
    major = int(match[1]) if match else 9
    if major <= 5:
        return "7.33.7"
    if major == 6:
        return "8.15.9"
    # pnpm 11 stopped reading package.json "pnpm" settings and pnpm 12 turns "workspaces" into a workspace file;
    # either changes the frozen-lockfile check for projects locked by pnpm 10.
    if "pnpm" in package or ("workspaces" in package and not (root / "pnpm-workspace.yaml").exists()):
        return "10.33.0"
    return "12.10.1"


def install_plan(root: Path) -> dict | None:
    path = root / "package.json"
    if not path.exists():
        return None
    package = json.loads(path.read_text())
    locked = [name for name, files in LOCKFILES.items() if any((root / f).exists() for f in files)]
    name, version = declared_manager(package) or ("", "")
    if name != "nub" and locked and name not in locked:
        name, version = locked[0], ""
    name = name or "npm"
    lock = bool(locked) if name == "nub" else name in locked
    frozen = ["--frozen-lockfile"] if lock else []
    if name == "pnpm":
        command = ["corepack", "pnpm@" + (version or default_pnpm(root, package)), "install",
                   *(frozen or ["--no-frozen-lockfile"]), "--ignore-scripts", "--ignore-pnpmfile",
                   "--config.engine-strict=false"]
    elif name == "yarn":
        version = version or ("4.9.2" if lock and "__metadata:" in (root / "yarn.lock").read_text() else "1.22.22")
        if int(version.split(".", 1)[0]) >= 2:
            flags = [*(["--immutable"] if lock else []), "--mode=skip-build"]
        else:
            flags = [*(frozen or ["--no-lockfile"]), "--ignore-scripts", "--non-interactive", "--ignore-engines",
                     "--ignore-platform"]
        command = ["corepack", "yarn@" + version, "install", *flags]
    elif name == "npm":
        command = [*(["corepack", "npm@" + version] if version else ["npm"]),
                   *(["ci"] if lock else ["install", "--package-lock=false"]),
                   "--ignore-scripts", "--no-audit", "--no-fund", "--engine-strict=false"]
    elif name == "bun":
        command = [name, "install", *frozen, "--ignore-scripts"]
        version = version or "1.4.3"
    elif name == "nub":
        command = [name, "install", *(frozen or ["--no-frozen-lockfile"]), "--ignore-scripts"]
    else:
        raise ValueError(f"Unsupported package manager: {name}@{version}")
    return {"manager": name, "command": command, "dependencies": "lockfile" if lock else "unpinned",
            **({"tool": (name, version)} if name in TOOLS else {})}


def tool_binary(name: str, version: str, log: Path, timeout: float, env: dict[str, str], cache: Path) -> Path:
    check_version(name, version)
    system = {"Linux": "linux", "Darwin": "darwin"}.get(platform.system())
    arch = TOOL_ARCH.get((name, platform.machine()))
    if not system or not arch:
        raise ValueError(f"No {name} build for {platform.system()} {platform.machine()}")
    package = TOOLS[name].format(os=system, arch=arch)
    prefix = cache / "tools" / re.sub(r"[^A-Za-z0-9.-]", "_", f"{package}@{version}")
    binary = prefix / "node_modules" / package / "bin" / name
    if not binary.is_file():
        prefix.mkdir(parents=True, exist_ok=True)
        result = execute(["npm", "install", "--prefix", str(prefix), "--no-save", "--package-lock=false",
                          "--ignore-scripts", "--no-audit", "--no-fund", f"{package}@{version}"], prefix, log, timeout, env)
        if result["exit"] or result["timeout"] or not binary.is_file():
            raise ValueError(f"Could not install {package}@{version}; see {log.name}")
        binary.chmod(0o755)  # nub's packages set the mode in a postinstall script, which installs here skip
    return binary


def read_compilation(result: dict, log: Path, root: Path, binary: Path) -> dict:
    text = log.read_text(errors="replace").partition("\n")[2].replace("\r\n", "\n")
    match = FILES.search(text)
    diagnostic_text = text[:match.start()] if match else text
    diagnostic_text = diagnostic_text.replace(str(root), "<repo>").replace(str(binary.parent), "<compiler>")
    return dict(result, files=int(match[1]) if match else 0, diagnostics=diagnostic_text.rstrip())


def failed(run: dict) -> bool:
    return run["timeout"] or run["exit"] not in (0, 1, 2)


def compare(go: dict, rust: dict) -> str:
    if failed(go):
        return "reference_failed"
    if failed(rust):
        return "tsrs_failed"
    if any(go[key] != rust[key] for key in ("exit", "files", "diagnostics")):
        return "mismatch"
    if not go["files"]:
        return "no_inputs"
    return "match"


def split_output(text: str) -> tuple[list[str], list[str]]:
    """Diagnostics (a header and its indented lines) in order, and the other lines, such as resolution traces."""
    diagnostics, other, inside = [], [], False
    for line in text.splitlines():
        if DIAGNOSTIC.match(line):
            diagnostics.append(line)
            inside = True
        elif inside and line.startswith("  "):
            diagnostics[-1] += "\n" + line
        else:
            other.append(line)
            inside = False
    return diagnostics, other


def difference(go: dict, rust: dict) -> list[str]:
    """What differs, to read a mismatch faster; any difference is still a mismatch."""
    parts = [name for key, name in (("exit", "exit code"), ("files", "loaded files")) if go[key] != rust[key]]
    for name, a, b in zip(("diagnostics", "other output"), split_output(go["diagnostics"]),
                          split_output(rust["diagnostics"])):
        if a != b:
            parts.append(name)
    return parts


def compile_project(root: Path, config: str, ctx: Context, out: Path, label: str, binary: Path, flags: list[str],
                    timeout: float) -> dict:
    info = out / (label + ".tsbuildinfo")
    command = [str(binary), "-p", config, *flags, "--noEmit", "--pretty", "false", "--extendedDiagnostics",
               "--incremental", "--tsBuildInfoFile", str(info)]
    info.unlink(missing_ok=True)
    result = execute(command, root, out / (label + ".log"), timeout, ctx.env)
    info.unlink(missing_ok=True)
    return dict(read_compilation(result, out / (label + ".log"), root, binary), limit=round(timeout, 1))


def outcome(go: dict, rust: dict | None, diff: Path) -> dict:
    status = "reference_failed" if rust is None else compare(go, rust)
    if status != "mismatch":
        return {"status": status}
    def lines(run: dict) -> list[str]:
        return [f"exit: {run['exit']}\n", f"files: {run['files']}\n", *(s + "\n" for s in run["diagnostics"].splitlines())]
    diff.write_text("".join(difflib.unified_diff(lines(go), lines(rust), fromfile="tsgo", tofile="tsrs")))
    return {"status": status, "difference": difference(go, rust)}


def summary(run: dict) -> dict:
    return {k: v for k, v in run.items() if k != "diagnostics"}


def check_config(root: Path, config: str, ctx: Context, out: Path, timeout: float) -> dict:
    """Default mode: both compilers as users run them, with the project's own options."""
    out = out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    flags = MODES["default"]
    go = compile_project(root, config, ctx, out, "tsgo", ctx.tsgo, flags["tsgo"], timeout)
    rust = None if failed(go) else compile_project(root, config, ctx, out, "tsrs", ctx.tsrs, flags["tsrs"], timeout)
    return {"config": config, **outcome(go, rust, out / "diagnostics.diff"), "limit": round(timeout, 1),
            "tsgo": summary(go), **({"tsrs": summary(rust)} if rust else {})}


def check_strict(root: Path, default: dict, ctx: Context, out: Path, budget: float) -> dict:
    """Strict mode with its own tsgo run: its reference command differs from the default one whenever the project
    sets "checkers", which no tsgo output reports. tsgo gets half of budget and tsrs the rest. A timeout with less
    time than the default run's limit is budget_exhausted (not compared), not a failure."""
    out = out.resolve()
    end = time.monotonic() + budget
    flags = MODES["strict"]
    go = compile_project(root, default["config"], ctx, out, "tsgo-strict", ctx.tsgo, flags["tsgo"], min(60, budget / 2))
    rust = None if failed(go) else compile_project(root, default["config"], ctx, out, "tsrs-strict", ctx.tsrs,
                                                   flags["tsrs"], min(60, max(0.01, end - time.monotonic())))
    result = {**outcome(go, rust, out / "diagnostics-strict.diff"), "tsgo": summary(go),
              **({"tsrs": summary(rust)} if rust else {})}
    if any(run and run["timeout"] and run["limit"] < default["limit"] for run in (go, rust)):
        result["status"] = "budget_exhausted"
    return result


def project_references(root: Path, config: str, ctx: Context, log: Path, timeout: float) -> list[str]:
    """Referenced config paths, resolved like tsgo's resolveProjectReferencePath, that exist in the checkout."""
    result = execute([str(ctx.tsgo), "-p", config, "--showConfig"], root, log, timeout, ctx.env)
    if result["exit"] or result["timeout"]:
        return []
    try:
        references = json.loads(log.read_text().partition("\n")[2]).get("references", [])
    except ValueError:
        return []
    paths = []
    for reference in references:
        path = os.path.normpath(os.path.join(os.path.dirname(config), reference.get("path", "")))
        if not path.endswith(".json"):
            path = os.path.join(path, "tsconfig.json")
        if not os.path.isabs(path) and not path.startswith("..") and (root / path).is_file():
            paths.append(Path(path).as_posix())
    return paths


def check_projects(root: Path, configs: list[str], ctx: Context, out: Path, check_timeout: float,
                   max_configs: int) -> dict:
    """Compare up to max_configs projects that load files. A config with no inputs of its own (a solution file)
    queues its project references next; at most 2 * max_configs configs are run."""
    checked, queue = [], list(configs)
    deadline = time.monotonic() + check_timeout
    while queue and len(checked) < 2 * max_configs:
        if sum(c["status"] not in ("no_inputs", "references") for c in checked) >= max_configs:
            break
        config = queue.pop(0)
        remaining = deadline - time.monotonic()
        if remaining < 1:
            checked.append({"config": config, "status": "budget_exhausted"})
            break
        index = out / str(len(checked))
        result = check_config(root, config, ctx, index, min(60, remaining / 2))
        if result["status"] == "no_inputs":
            remaining = deadline - time.monotonic()
            done = {c["config"] for c in checked} | {config}
            references = [r for r in project_references(root, config, ctx, index / "references.log",
                                                         min(10, max(remaining, 0.01))) if r not in done]
            if references:
                result.update(status="references", references=references)
                queue = references + [q for q in queue if q not in references]
        checked.append(result)
    counts = Counter(c["status"] for c in checked)
    if counts["mismatch"] or counts["tsrs_failed"]:
        status = "mismatch" if counts["mismatch"] else "tsrs_failed"
    elif counts["match"]:
        status = "match" if not queue and set(counts) <= {"match", "references"} else "partial"
    else:
        status = "not_checked"
    # Strict mode runs after every default comparison so that it never takes their time, and splits what is left.
    pending = [(i, c) for i, c in enumerate(checked) if c["status"] in COMPARED]
    for n, (i, config) in enumerate(pending):
        remaining = deadline - time.monotonic()
        config["strict"] = (check_strict(root, config, ctx, out / str(i), remaining / (len(pending) - n))
                            if remaining >= 1 else {"status": "budget_exhausted"})
    strict = Counter(c["strict"]["status"] for _, c in pending)
    if strict["mismatch"] or strict["tsrs_failed"]:
        strict_status = "mismatch" if strict["mismatch"] else "tsrs_failed"
    elif strict["match"]:
        strict_status = "match" if strict["match"] == len(pending) else "partial"
    else:
        strict_status = "not_checked"
    return {"status": status, "strict_status": strict_status, "configs": checked}


def check_repo(repo: dict, tsgo: Path, tsrs: Path, out: Path, cache: Path, setup_timeout: int,
               check_timeout: int, max_configs: int, source: Path | None = None) -> dict:
    result = dict(repo, status="setup_failed", configs=[])
    with tempfile.TemporaryDirectory(prefix="tsrs-repo-") as temp:
        root = Path(temp) / "repo"
        ctx = Context(tsgo, tsrs, child_env(Path(temp) / "home", cache))
        try:
            deadline = time.monotonic() + setup_timeout
            url = str(source) if source else "https://github.com/" + repo["repository"] + ".git"
            if repo.get("commit"):
                clone = ["sh", "-c", 'git init -q "$1" && git -C "$1" fetch -q --depth 1 "$2" "$3" && '
                         'git -C "$1" checkout -q FETCH_HEAD', "clone", str(root), url, repo["commit"]]
            else:
                clone = ["git", "clone", "--depth", "1", "--single-branch", "--branch", repo["branch"], url, str(root)]
            cloned = execute(clone, Path(temp), out / "clone.log", min(60, setup_timeout), ctx.env)
            if cloned["exit"] or cloned["timeout"]:
                result["reason"] = f"Clone {'timed out' if cloned['timeout'] else 'failed'}; see clone.log"
                return result
            result["commit"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, env=ctx.env,
                                                       text=True).strip()
            tracked = subprocess.check_output(["git", "ls-files", "-z"], cwd=root, env=ctx.env).decode().split("\0")
            configs = sorted((p for p in tracked if Path(p).name == "tsconfig.json" and "node_modules" not in Path(p).parts),
                             key=lambda p: (len(Path(p).parts), p))
            result["available_configs"] = len(configs)
            if not configs:
                result.update(status="no_config", reason="No tracked tsconfig.json")
                return result
            plan = install_plan(root)
            if plan:
                result["dependencies"] = plan["dependencies"]
                command = plan["command"]
                if "tool" in plan:
                    binary = tool_binary(*plan["tool"], out / "tool.log", max(0.01, deadline - time.monotonic()),
                                         ctx.env, cache)
                    command = [str(binary), *command[1:]]
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    result["reason"] = "Dependency setup budget exhausted"
                    return result
                install = execute(command, root, out / "install.log", remaining, ctx.env)
                result["install"] = dict(install, command=" ".join(plan["command"]))
                if install["exit"] or install["timeout"]:
                    result["reason"] = (f"Dependency installation {'timed out' if install['timeout'] else 'failed'}"
                                        f" ({plan['manager']}); see install.log")
                    return result
            result.update(check_projects(root, configs, ctx, out, check_timeout, max_configs))
            return result
        except (OSError, ValueError, subprocess.CalledProcessError) as error:
            result["reason"] = str(error)
            return result


def run_shard(args: argparse.Namespace, manifest: dict) -> None:
    from sandbox import check_in_container

    if not 0 <= args.shard < manifest["shards"]:
        raise ValueError("Shard outside manifest range")
    for binary in (args.tsgo, args.tsrs):
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError(f"Compiler is not executable: {binary}")
    results = []
    path = args.out / f"shard-{args.shard}.json"
    report = {"tsrs_commit": manifest["tsrs_commit"], "typescript_commit": manifest["typescript_commit"],
              "shard": args.shard, "setup_timeout": args.setup_timeout, "check_timeout": args.check_timeout,
              "max_configs": args.max_configs, "modes": MODES, "results": results}
    write_json(path, report)
    for repo in manifest["repositories"][args.shard::manifest["shards"]]:
        name = repo["repository"]
        result = check_in_container(repo, args.tsgo.resolve(), args.tsrs.resolve(), args.out / name.replace("/", "--"),
                                    args.cache.resolve(), args.setup_timeout, args.check_timeout, args.max_configs,
                                    args.sandbox_image)
        results.append(result)
        write_json(path, report)
        print(f"{name}: {result['status']}", flush=True)


# Settings every merged shard report must share, including the modes and their flags.
SETTINGS = ("setup_timeout", "check_timeout", "max_configs", "modes")


def load_shards(directory: Path, manifest: dict, errors: list[str]) -> dict[int, dict]:
    """Shard reports by index, keeping only results for repositories the manifest assigns to that shard. Reports from
    other compilers or with unknown modes are left out, so their repositories count as missing."""
    reports = {}
    for path in sorted(directory.rglob("shard-*.json")):
        report = json.loads(path.read_text())
        report.setdefault("modes", LEGACY_MODES)
        shard = report["shard"]
        if shard in reports or not 0 <= shard < manifest["shards"]:
            errors.append(f"{path}: duplicate or unexpected shard {shard}")
            continue
        wrong = [key for key in ("tsrs_commit", "typescript_commit") if report[key] != manifest[key]]
        if wrong:
            errors.append(f"{path}: {' and '.join(wrong)} do not match the manifest; its results are left out")
            continue
        if report["modes"] not in (MODES, LEGACY_MODES):
            errors.append(f"{path}: unknown comparison modes {json.dumps(report['modes'])}; its results are left out")
            continue
        expected = {r["repository"] for r in manifest["repositories"][shard::manifest["shards"]]}
        outside = [r["repository"] for r in report["results"] if r["repository"] not in expected]
        if outside:
            errors.append(f"{path}: results for repositories outside shard {shard}: {', '.join(outside)}")
        report["results"] = [r for r in report["results"] if r["repository"] in expected]
        reports[shard] = dict(report, path=str(path))
    return reports


def detail(result: dict) -> str:
    if result.get("reason"):
        return result["reason"]
    notes = []
    for config in result["configs"]:
        status = config["status"]
        if status == "reference_failed":
            go = config["tsgo"]
            limit = go.get("limit", config.get("limit"))
            status += f" (tsgo {'timed out at ' + str(limit) + 's' if go['timeout'] else 'exit ' + str(go['exit'])})"
        elif config.get("difference"):
            status += f" ({', '.join(config['difference'])})"
        note = status if status not in ("match", "references") else ""
        strict = config.get("strict")
        if strict and (strict["status"] != "match" or note):
            note = f"{note or 'match'}; strict {strict['status']}" + (
                f" ({', '.join(strict['difference'])})" if strict.get("difference") else "")
        if note:
            notes.append(f"{config['config']}: {note}")
    if result.get("dependencies") == "unpinned":
        notes.append("no lockfile; dependencies unpinned")
    return ", ".join(notes)


def command(flags: list[str]) -> str:
    return " ".join(flags) or "no extra flags"


def at_pin(result: dict, pinned: dict[str, str | None]) -> bool:
    # A clone that failed before checking anything out is the only result without the pinned commit.
    unchecked = result.get("commit") is None and result["status"] == "setup_failed" and not result["configs"]
    pin = pinned[result["repository"]]
    return not pin or result.get("commit") == pin or unchecked


def differences(result: dict) -> list[str]:
    """Each project's default or strict difference or tsrs failure, as "config: mode status"."""
    found = [f"{config['config']}: {mode} {entry['status']}"
             for config in result.get("configs", [])
             for mode, entry in (("default", config), ("strict", config.get("strict") or {}))
             if entry.get("status") in ("mismatch", "tsrs_failed")]
    return found or [f"{mode} {result[key]}" for mode, key in (("default", "status"), ("strict", "strict_status"))
                     if result.get(key) in ("mismatch", "tsrs_failed")]


def replace_shards(reports: dict[int, dict], earlier: dict[int, dict], manifest: dict, pinned: dict[str, str | None],
                   errors: list[str]) -> None:
    """Fill reports with the earlier run's shards. A rerun only replaces a shard that was lost or cut short: a complete
    earlier shard is kept and the rerun refused, and differences and tsrs failures already recorded for a partial
    shard stay errors, so rerunning cannot clear them."""
    for shard, report in sorted(earlier.items()):
        if shard not in reports:
            reports[shard] = report
            continue
        assigned = {r["repository"] for r in manifest["repositories"][shard::manifest["shards"]]}
        if assigned <= {r["repository"] for r in report["results"]}:
            errors.append(f"Shard {shard} was complete in the earlier run ({report['path']}); a rerun replaces only "
                          "lost or partial shards, so the earlier report is kept")
            reports[shard] = report
            continue
        for result in report["results"]:
            if at_pin(result, pinned) and differences(result):
                errors.append(f"{result['repository']}: the earlier run's shard {shard} ({report['path']}) recorded "
                              f"{', '.join(differences(result))}; a rerun does not clear it")


def summarize(manifest: dict, directory: Path, out: Path, previous: Path | None = None) -> bool:
    """Write the report. Shards missing from directory are taken from previous, a run with the same manifest."""
    pinned = {r["repository"]: r.get("commit") for r in manifest["repositories"]}
    errors: list[str] = []
    reports = load_shards(directory, manifest, errors)
    current = dict(reports)
    if previous:
        replace_shards(reports, load_shards(previous, manifest, errors), manifest, pinned, errors)
    if reports:
        base = min(current or reports)
        for shard, report in sorted(reports.items()):
            changed = [f"{k} {json.dumps(report.get(k))} (shard {base}: {json.dumps(reports[base].get(k))})"
                       for k in SETTINGS if report.get(k) != reports[base].get(k)]
            if changed:
                errors.append(f"Shard {shard} used other settings: {', '.join(changed)}; its results are left out")
                del reports[shard]
    modes = reports[min(reports)]["modes"] if reports else MODES
    results = {}
    for report in reports.values():
        for result in report["results"]:
            name = result["repository"]
            if name in results:
                errors.append(f"Duplicate result for {name}")
            elif not at_pin(result, pinned):
                errors.append(f"{name}: checked {result.get('commit')}, manifest pins {pinned[name]}")
            else:
                results[name] = result
    errors.extend(f"Missing repository result: {name}" for name in sorted(pinned.keys() - results.keys()))
    if set(reports) != set(range(manifest["shards"])):
        errors.append("Some shard reports are missing")
    counts = Counter(r["status"] for r in results.values())
    configs = [c for r in results.values() for c in r["configs"]]
    projects = Counter(c["status"] for c in configs)
    completed = projects["match"] + projects["mismatch"]
    clean = sum(c["status"] == "match" and c["tsgo"]["exit"] == 0 for c in configs)
    repositories = sum(any(c["status"] in ("match", "mismatch") for c in r["configs"]) for r in results.values())
    eligible = [c for c in configs if c["status"] in COMPARED]
    strict = Counter(c["strict"]["status"] for c in eligible if "strict" in c)
    strict_only = sum(c["status"] != "match" and c.get("strict", {}).get("status") == "match" for c in eligible)
    if not completed:
        errors.append("No project completed a comparison")
    if "strict" in modes:
        strict_lines = [
            f"tsgo `{command(modes['strict']['tsgo'])}` vs tsrs `{command(modes['strict']['tsrs'])}`, both run again "
            "after all default comparisons, in the time they leave. A difference here fails the report too, and "
            "strict-mode identity does not excuse a default-mode difference.", "",
            f"Of the {len(eligible)} projects where tsgo completed in default mode: {strict['match']} identical, "
            f"{strict['mismatch']} different, {strict['tsrs_failed']} tsrs failures in strict mode; strict tsgo failed "
            f"on {strict['reference_failed']} and {strict['budget_exhausted']} were not run for lack of time. "
            f"{strict_only} projects that differ or fail in default mode are identical in strict mode."]
    else:
        strict_lines = ["These shard reports predate strict mode; only default mode was compared."]
    lines = ["# Top repository conformance", "",
             f"tsrs `{manifest['tsrs_commit']}` vs tsgo from TypeScript `{manifest['typescript_commit']}`.", "",
             "## Default mode", "",
             f"tsgo `{command(modes['default']['tsgo'])}` vs tsrs `{command(modes['default']['tsrs'])}`, with each "
             "project's own options.", "",
             f"{len(results)} of {len(pinned)} repositories have a result; {repositories} had at least one project "
             "where both compilers completed.", "",
             f"Both compilers completed on {completed} projects: {projects['match']} identical ({clean} without errors, "
             f"{projects['match'] - clean} with the same errors) and {projects['mismatch']} different. tsrs crashed or "
             f"timed out on {projects['tsrs_failed']} more after tsgo completed; tsgo failed or timed out on "
             f"{projects['reference_failed']}.", "",
             "Identical means the same output in the same order (diagnostics and other lines, such as "
             "`traceResolution` traces), the same exit code and the same nonzero number of loaded files. It does not "
             "mean the project has no type errors.", "",
             "Repository status: " + ", ".join(f"{status}: {n}" for status, n in sorted(counts.items())) + ".", "",
             "Project status: " + ", ".join(f"{status}: {n}" for status, n in sorted(projects.items())) + ".", "",
             "`partial`: some projects were identical and others were not compared (limits, no inputs, or tsgo "
             "failures). `references`: a solution config with no files of its own; its referenced projects were "
             "compared instead. `no_inputs`: neither compiler loaded a file.", "",
             "A difference lists what differs: exit code, loaded files, diagnostics, or other output (lines that are "
             "not diagnostics, such as `traceResolution` traces). Any of these fails the report.", "",
             "## Strict mode", "", *strict_lines, "",
             "## Repositories", "",
             "Checks use `--noEmit`. Artifacts contain logs, diffs, and repository commits. Projects include configs "
             "reached through project references, which need not be tracked `tsconfig.json` files.", "",
             "| Repository | Commit | Status | Strict | Identical / completed projects | Tracked tsconfig.json | Detail |",
             "| --- | --- | --- | --- | --- | --- | --- |"]
    for name in sorted(pinned):
        r = results.get(name, {"status": "missing", "configs": []})
        matching = sum(c["status"] == "match" for c in r["configs"])
        done = sum(c["status"] in ("match", "mismatch") for c in r["configs"])
        text = detail(r).replace("|", "\\|").replace("\n", " ")[:500]
        lines.append(f"| [{name}](https://github.com/{name}) | `{r.get('commit') or pinned[name] or ''}` | {r['status']} | "
                     f"{r.get('strict_status', '')} | {matching} / {done} | {r.get('available_configs', 0)} | {text} |")
    if errors:
        lines.extend(["", "## Incomplete run", "", *("- " + e for e in errors)])
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(lines) + "\n")
    return not errors and not any(counter[s] for counter in (projects, strict) for s in ("mismatch", "tsrs_failed"))


def parse_shards(text: str, count: int) -> list[int]:
    if not re.fullmatch(r"\s*\d+\s*(,\s*\d+\s*)*", text):
        raise ValueError(f"Shards must be comma-separated indexes, not {text!r}")
    shards = sorted({int(s) for s in text.split(",")})
    if shards[-1] >= count:
        raise ValueError(f"Shards must be between 0 and {count - 1}")
    return shards


def check_dispatch(run_id: str, shards: str) -> None:
    """Rerun inputs come as a pair, so a malformed recovery request fails instead of starting a full run."""
    if bool(run_id) != bool(shards):
        raise ValueError("Set rerun_run_id and rerun_shards together")
    if run_id and not re.fullmatch(r"[1-9]\d{0,19}", run_id):
        raise ValueError(f"rerun_run_id must be a workflow run id, not {run_id!r}")
    if shards:
        parse_shards(shards, MAX_SHARDS)


def check_rerun(manifest: dict, shards: str, run: dict, inputs: Path) -> list[int]:
    """The shards to rerun, once the manifest is known to reproduce them: written by this harness version for that
    run, with pinned repository and compiler commits and the run's compilers."""
    if "recovery_of_run_id" in manifest:
        raise ValueError(f"Run {run.get('id')} is a recovery; use the original run {manifest['recovery_of_run_id']} "
                         "and include all shards still needing recovery")
    if manifest.get("harness") != HARNESS:
        raise ValueError(f"The manifest is from harness version {manifest.get('harness')}, not {HARNESS}; older "
                         "manifests may not pin repository commits")
    if not all(SHA.fullmatch(str(r.get("commit"))) for r in manifest["repositories"]):
        raise ValueError("The manifest does not pin every repository commit")
    if not all(SHA.fullmatch(str(manifest.get(key))) for key in ("tsrs_commit", "typescript_commit")):
        raise ValueError("The manifest does not pin the compiler commits")
    if str(run.get("path", "")).split("@")[0] != WORKFLOW:
        raise ValueError(f"Run {run.get('id')} is not a {WORKFLOW} run")
    # A fork pull request's compilers would run here and their cache would be saved for later runs to restore.
    head, base = ((run.get(key) or {}).get("full_name") for key in ("head_repository", "repository"))
    if not head or head != base:
        raise ValueError(f"Run {run.get('id')} was built from {head}, not {base}")
    # A pull request run's manifest names the merge commit it checked out, not the run's head.
    if run.get("event") != "pull_request" and run.get("head_sha") != manifest["tsrs_commit"]:
        raise ValueError(f"Run {run.get('id')} is at {run.get('head_sha')}, the manifest at {manifest['tsrs_commit']}")
    for name in ("tsgo", "tsrs"):
        if not (inputs / "bin" / name).is_file():
            raise ValueError(f"The run's inputs have no {name} binary")
    return parse_shards(shards, manifest["shards"])


def write_matrix(shards: list[int]) -> None:
    matrix = json.dumps({"shard": shards})
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as output:
            output.write("matrix=" + matrix + "\n")
    print(matrix)


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
    run.add_argument("--cache", type=Path, required=True, help="scratch directory; each container gets a private cache")
    run.add_argument("--sandbox-image", default="tsrs-repo-conformance", help="locally built Docker image; required")
    run.add_argument("--setup-timeout", type=positive, default=180)
    run.add_argument("--check-timeout", type=positive, default=120)
    run.add_argument("--max-configs", type=positive, default=5)
    report = commands.add_parser("report")
    report.add_argument("--manifest", type=Path, required=True)
    report.add_argument("--results", type=Path, required=True)
    report.add_argument("--out", type=Path, required=True)
    report.add_argument("--previous", type=Path, help="shard reports of an earlier run with the same manifest")
    rerun = commands.add_parser("rerun", help="print the matrix for shards to rerun from a saved manifest")
    rerun.add_argument("--manifest", type=Path, required=True)
    rerun.add_argument("--shards", required=True, help="comma-separated shard indexes")
    rerun.add_argument("--run", type=Path, required=True, help="the earlier run's GitHub API object")
    dispatch = commands.add_parser("dispatch", help="check the workflow_dispatch rerun inputs")
    dispatch.add_argument("--run-id", default="")
    dispatch.add_argument("--shards", default="")
    commands.add_parser("_repo", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.command == "plan":
        if args.count > 400 or args.shards > MAX_SHARDS:
            parser.error("At most 400 repositories and 20 shards")
        manifest = discover(args.count, args.shards, args.out)
        write_matrix(list(range(manifest["shards"])))
    elif args.command == "dispatch":
        try:
            check_dispatch(args.run_id, args.shards)
        except ValueError as error:
            parser.error(str(error))
    elif args.command == "rerun":
        try:
            manifest = json.loads(args.manifest.read_text())
            earlier = json.loads(args.run.read_text())
            shards = check_rerun(manifest, args.shards, earlier, args.manifest.parent)
            write_json(args.manifest, dict(manifest, recovery_of_run_id=earlier["id"]))
            write_matrix(shards)
        except ValueError as error:
            parser.error(str(error))
    elif args.command == "run":
        run_shard(args, json.loads(args.manifest.read_text()))
    elif args.command == "_repo":
        request = json.loads(Path("/request.json").read_text())
        result = check_repo(request["repo"], Path("/inputs/tsgo"), Path("/inputs/tsrs"), Path("/work/out"),
                            Path("/work/cache"), request["setup_timeout"], request["check_timeout"],
                            request["max_configs"], Path("/source") if request.get("source") else None)
        write_json(Path("/work/result.json"), result)
    else:
        return 0 if summarize(json.loads(args.manifest.read_text()), args.results, args.out, args.previous) else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
