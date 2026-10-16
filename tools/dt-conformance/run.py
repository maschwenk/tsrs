#!/usr/bin/env python3
"""Compare pinned tsgo and tsrs on every DefinitelyTyped package through dtslint."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import subprocess
import sys
import traceback


ROOT = Path(__file__).resolve().parents[2]
DT_REPOSITORY = "https://github.com/DefinitelyTyped/DefinitelyTyped.git"
SHA = re.compile(r"[0-9a-f]{40}\Z")
PACKAGE_MANAGER = re.compile(r"pnpm@(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+sha\d+\.[0-9a-fA-F]+)?)\Z")
START = re.compile(r"^(?:\d+> )?(\S+) START$")
FINISH = re.compile(r"^(?:\d+> )?(\S+) (?:OK(?: \(with warnings\))?|failing:|Out of memory: failed)$")
HARNESS = 1


def write_json(path: Path, data: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n")


def child_env(home: Path, cache: Path) -> dict[str, str]:
    """Environment exposed to the DT install, dtslint and compiler processes."""
    allowed = {"PATH", "LANG", "LC_ALL", "USER", "LOGNAME"}
    env = {key: value for key, value in os.environ.items() if key in allowed}
    runtime = home / ".runtime"
    runtime.mkdir(parents=True, exist_ok=True, mode=0o700)
    for path in (home / ".npmrc", home / ".gitconfig"):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.touch()
    return dict(
        env,
        HOME=str(home),
        TMPDIR="/work/tmp",
        XDG_RUNTIME_DIR=str(runtime),
        XDG_CONFIG_HOME=str(home / ".config"),
        XDG_STATE_HOME=str(home / ".state"),
        XDG_CACHE_HOME=str(cache / "xdg"),
        XDG_DATA_HOME=str(cache / "data"),
        COREPACK_HOME=str(cache / "corepack"),
        npm_config_cache=str(cache / "npm"),
        npm_config_userconfig=str(home / ".npmrc"),
        npm_config_ignore_scripts="true",
        GIT_CONFIG_GLOBAL=str(home / ".gitconfig"),
        GIT_CONFIG_NOSYSTEM="1",
        GIT_TERMINAL_PROMPT="0",
        COREPACK_ENABLE_DOWNLOAD_PROMPT="0",
        COREPACK_ENABLE_PROJECT_SPEC="0",
        CI="true",
    )


def execute(command: list[str], cwd: Path, log: Path, timeout: float, env: dict[str, str], append: bool = False) -> dict:
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("a" if append else "w") as output:
        output.write("$ " + shlex.join(command) + "\n")
        output.flush()
        process = subprocess.Popen(command, cwd=cwd, env=env, stdout=output, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        try:
            code = process.wait(timeout=max(timeout, 0.01))
            timed_out = False
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            code = process.wait()
            timed_out = True
    return {"exit": code, "timeout": timed_out}


def resolve_dt_commit(commit: str) -> str:
    if commit:
        if not SHA.fullmatch(commit):
            raise ValueError("DefinitelyTyped commit must be a full 40-character lowercase SHA")
        return commit
    output = subprocess.check_output(
        ["git", "ls-remote", DT_REPOSITORY, "refs/heads/master"], text=True, timeout=60,
    ).split()
    if not output or not SHA.fullmatch(output[0]):
        raise RuntimeError("Could not resolve DefinitelyTyped master")
    return output[0]


def create_manifest(commit: str, out: Path) -> dict:
    manifest = {
        "schema": 1,
        "harness": HARNESS,
        "created_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "definitely_typed_commit": resolve_dt_commit(commit),
        "typescript_commit": subprocess.check_output(
            ["node", "npm/build.mjs", "--print-typescript-commit"], cwd=ROOT, text=True,
        ).strip(),
        "tsrs_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
    }
    write_json(out, manifest)
    return manifest


def package_progress(log: Path) -> tuple[set[str], set[str]]:
    started: set[str] = set()
    finished: set[str] = set()
    for line in log.read_text(errors="replace").splitlines():
        if match := START.fullmatch(line):
            started.add(match[1])
        elif match := FINISH.fullmatch(line):
            finished.add(match[1])
    return started, finished


def normalize_text(text: str, replacements: dict[Path, str]) -> str:
    text = text.replace("\r\n", "\n")
    for path, replacement in sorted(replacements.items(), key=lambda item: len(str(item[0])), reverse=True):
        text = text.replace(str(path), replacement)
    return text


def read_failures(path: Path, replacements: dict[Path, str]) -> dict[str, str]:
    value = json.loads(path.read_text())
    if not isinstance(value, list):
        raise ValueError(f"{path} is not a failure array")
    failures: dict[str, str] = {}
    for entry in value:
        if not isinstance(entry, dict) or not isinstance(entry.get("path"), str) or not isinstance(entry.get("error"), str):
            raise ValueError(f"{path} contains an invalid failure")
        if entry["path"] in failures:
            raise ValueError(f"{path} contains duplicate package {entry['path']}")
        failures[entry["path"]] = normalize_text(entry["error"], replacements)
    return dict(sorted(failures.items()))


def validate_run(run: dict, failures: dict[str, str], started: set[str], finished: set[str], label: str) -> None:
    if run["timeout"]:
        raise RuntimeError(f"{label} dtslint run timed out")
    if run["exit"] < 0:
        raise RuntimeError(f"{label} dtslint run ended by signal {-run['exit']}")
    if not started or started != finished:
        missing = sorted(started - finished)[:20]
        unexpected = sorted(finished - started)[:20]
        raise RuntimeError(f"{label} package coverage was incomplete: missing={missing}, unexpected={unexpected}")
    expected_exit = len(failures) % 256
    if run["exit"] != expected_exit:
        raise RuntimeError(
            f"{label} dtslint exited {run['exit']}, expected {expected_exit} for {len(failures)} failures"
        )


def compare_failures(reference: dict[str, str], candidate: dict[str, str]) -> dict:
    reference_names = set(reference)
    candidate_names = set(candidate)
    changed = sorted(name for name in reference_names & candidate_names if reference[name] != candidate[name])
    return {
        "reference_only": sorted(reference_names - candidate_names),
        "candidate_only": sorted(candidate_names - reference_names),
        "changed": changed,
    }


def markdown_report(result: dict) -> str:
    status = result["status"]
    lines = [
        "# DefinitelyTyped conformance",
        "",
        f"**{status.replace('_', ' ').title()}**",
        "",
        f"- DefinitelyTyped: `{result.get('definitely_typed_commit', 'unknown')}`",
        f"- TypeScript: `{result.get('typescript_commit', 'unknown')}`",
        f"- tsrs: `{result.get('tsrs_commit', 'unknown')}`",
    ]
    if status in ("match", "mismatch"):
        lines += [
            f"- Packages completed: {result['packages']}",
            f"- Reference failures: {result['reference_failures']}",
            f"- Candidate failures: {result['candidate_failures']}",
            f"- dtslint-runner: `{result['dtslint_runner_version']}`",
            "",
        ]
        diffs = result["differences"]
        if status == "match":
            lines.append("Every package produced the same result and failure text.")
        else:
            labels = (("Reference only", "reference_only"), ("Candidate only", "candidate_only"),
                      ("Changed failure text", "changed"), ("Coverage difference", "coverage"))
            for title, key in labels:
                values = diffs.get(key, [])
                if values:
                    lines += [f"## {title}", "", *[f"- `{name}`" for name in values[:100]], ""]
                    if len(values) > 100:
                        lines.append(f"Full list: `differences.json` ({len(values)} entries).")
    else:
        lines += ["", result.get("reason", "The suite did not complete."), ""]
    return "\n".join(lines).rstrip() + "\n"


def run_dtslint(label: str, dt_path: Path, package_path: Path, binary_override: Path | None,
                compiler_root: Path, runner: Path, out: Path, timeout: int, processes: int,
                env: dict[str, str]) -> tuple[dict, dict[str, str], set[str]]:
    failures_path = out / f"{label}-failures.raw.json"
    log = out / f"{label}.log"
    command = [
        "node", "--enable-source-maps", str(runner), "--path", str(dt_path),
        "--localTypeScriptPath", str(package_path), "--selection", "all", "--expectOnly", "--noInstall",
        "--nProcesses", str(processes), "--writeFailures", str(failures_path),
    ]
    run_env = dict(env)
    run_env.pop("TSRS_BINARY", None)
    if binary_override:
        run_env["TSRS_BINARY"] = str(binary_override)
    run = execute(command, dt_path, log, timeout, run_env)
    if not failures_path.is_file():
        raise RuntimeError(f"{label} dtslint did not write {failures_path.name}")
    replacements = {
        dt_path: "<DefinitelyTyped>",
        package_path: "<TypeScript package>",
        compiler_root: "<compiler>",
        binary_override or compiler_root / "tsc": "<compiler executable>",
    }
    failures = read_failures(failures_path, replacements)
    write_json(out / f"{label}-failures.json", [{"path": path, "error": error} for path, error in failures.items()])
    started, finished = package_progress(log)
    validate_run(run, failures, started, finished, label)
    return run, failures, finished


def install_definitely_typed(commit: str, work: Path, out: Path, timeout: int, env: dict[str, str]) -> Path:
    dt_path = work / "DefinitelyTyped"
    setup_log = out / "setup.log"
    dt_path.mkdir()
    commands = [
        (["git", "init", "--quiet", "--initial-branch=master", "--template=", str(dt_path)], work),
        (["git", "remote", "add", "origin", DT_REPOSITORY], dt_path),
        (["git", "fetch", "--quiet", "--depth", "1", "--filter=blob:none", "origin", commit], dt_path),
        (["git", "checkout", "--quiet", "--detach", "FETCH_HEAD"], dt_path),
    ]
    for index, (command, cwd) in enumerate(commands):
        result = execute(command, cwd, setup_log, timeout, env, append=index > 0)
        if result["timeout"] or result["exit"]:
            raise RuntimeError(f"DefinitelyTyped checkout failed; see {setup_log.name}")
    package = json.loads((dt_path / "package.json").read_text())
    manager = package.get("packageManager", "")
    if not PACKAGE_MANAGER.fullmatch(manager):
        raise RuntimeError(f"DefinitelyTyped declares unsupported package manager {manager!r}")
    # DT intentionally sets `lockfile: false`; TypeScript's own DT pipeline also performs an unfrozen install.
    # The package-manager version is still pinned, and both compiler runs share this one dependency tree.
    install = execute(["corepack", manager, "install", "--ignore-scripts"], dt_path,
                      setup_log, timeout, env, append=True)
    if install["timeout"] or install["exit"]:
        raise RuntimeError(f"DefinitelyTyped install failed; see {setup_log.name}")
    return dt_path


def check_inputs(manifest: dict) -> None:
    if manifest.get("schema") != 1 or manifest.get("harness") != HARNESS:
        raise ValueError("Unsupported DefinitelyTyped conformance manifest")
    for key in ("definitely_typed_commit", "typescript_commit", "tsrs_commit"):
        if not SHA.fullmatch(str(manifest.get(key, ""))):
            raise ValueError(f"Manifest has invalid {key}")


def run_suite(request: dict) -> dict:
    work = Path("/work")
    out = work / "out"
    out.mkdir(parents=True, exist_ok=True)
    manifest = request["manifest"]
    check_inputs(manifest)
    write_json(out / "manifest.json", manifest)
    reference_root = Path("/inputs/typescript")
    reference_package = reference_root / "packages" / "typescript"
    reference_binary = reference_root / "built" / "local" / "tsc"
    candidate_package = Path("/inputs/tsrs-package")
    candidate_binary = Path("/inputs/tsrs-platform/tsrs")
    for path in (reference_package / "package.json", reference_binary, candidate_package / "package.json",
                 candidate_binary):
        if not path.exists():
            raise RuntimeError(f"Missing suite input {path}")
    env = child_env(work / "home", work / "cache")
    dt_path = install_definitely_typed(manifest["definitely_typed_commit"], work, out,
                                       request["setup_timeout"], env)
    runner = dt_path / "node_modules" / "@definitelytyped" / "dtslint-runner" / "dist" / "index.js"
    if not runner.is_file():
        raise RuntimeError("DefinitelyTyped install did not provide dtslint-runner")
    runner_root = runner.resolve().parent.parent
    runner_package = json.loads((runner_root / "package.json").read_text())
    expected = runner_root / "expectedFailures.txt"
    if not expected.is_file():
        raise RuntimeError("dtslint-runner has no expectedFailures.txt")
    shutil.copyfile(expected, out / "upstream-expected-failures.txt")
    expected.write_text("")
    reference_run, reference, reference_packages = run_dtslint(
        "reference", dt_path, reference_package, None, reference_binary.parent, runner, out,
        request["check_timeout"], request["processes"], env,
    )
    candidate_run, candidate, candidate_packages = run_dtslint(
        "candidate", dt_path, candidate_package, candidate_binary, candidate_binary.parent, runner, out,
        request["check_timeout"], request["processes"], env,
    )
    differences = compare_failures(reference, candidate)
    differences["coverage"] = sorted(reference_packages ^ candidate_packages)
    write_json(out / "differences.json", differences)
    status = "match" if not any(differences.values()) else "mismatch"
    result = {
        "status": status,
        **{key: manifest[key] for key in ("definitely_typed_commit", "typescript_commit", "tsrs_commit")},
        "packages": len(reference_packages),
        "reference_failures": len(reference),
        "candidate_failures": len(candidate),
        "dtslint_runner_version": runner_package["version"],
        "reference_run": reference_run,
        "candidate_run": candidate_run,
        "differences": differences,
    }
    write_json(out / "result.json", result)
    (out / "report.md").write_text(markdown_report(result))
    write_json(work / "result.json", result)
    return result


def failed_suite(error: BaseException) -> dict:
    work = Path("/work")
    out = work / "out"
    out.mkdir(parents=True, exist_ok=True)
    request = json.loads(Path("/request.json").read_text())
    manifest = request.get("manifest", {})
    result = {
        "status": "setup_failed",
        **{key: manifest.get(key) for key in ("definitely_typed_commit", "typescript_commit", "tsrs_commit")},
        "reason": str(error),
    }
    (out / "error.txt").write_text(traceback.format_exc())
    write_json(out / "result.json", result)
    (out / "report.md").write_text(markdown_report(result))
    write_json(work / "result.json", result)
    return result


def run_probe(request: dict) -> dict:
    """Synthetic hostile workload used by the required container-boundary test."""
    work = Path("/work")
    out = work / "out"
    out.mkdir(parents=True, exist_ok=True)
    token = Path(request["probe"]["host_token"])

    def blocked(function) -> bool:
        try:
            function()
            return False
        except (OSError, PermissionError):
            return True

    status = Path("/proc/self/status").read_text()
    evidence = {
        "host_token": blocked(token.read_text),
        "parent_root": blocked(lambda: Path("/proc/1/root" + str(token)).read_text()),
        "reference": blocked(lambda: Path("/inputs/typescript/package.json").write_text("poisoned")),
        "candidate": blocked(lambda: Path("/inputs/tsrs-package/package.json").write_text("poisoned")),
        "platform": blocked(lambda: Path("/inputs/tsrs-platform/tsrs").write_text("poisoned")),
        "harness": blocked(lambda: Path("/harness/tools/dt-conformance/run.py").write_text("poisoned")),
        "docker_socket": not Path("/var/run/docker.sock").exists(),
        "non_root": os.getuid() != 0,
        "no_new_privileges": bool(re.search(r"^NoNewPrivs:\s+1$", status, re.MULTILINE)),
        "no_capabilities": bool(re.search(r"^CapEff:\s+0+$", status, re.MULTILINE)),
        "no_runner_token": not os.environ.get("ACTIONS_RUNTIME_TOKEN") and not os.environ.get("GITHUB_TOKEN"),
    }
    try:
        (out / "host-link").symlink_to(token)
    except OSError:
        pass
    child = (
        "import time\nfrom pathlib import Path\np=Path('/work/out/heartbeat')\n"
        "while True:\n p.open('a').write('.')\n time.sleep(0.02)\n"
    )
    subprocess.Popen([sys.executable, "-c", child], start_new_session=True, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL)
    write_json(out / "probe.json", evidence)
    result = {"status": "match" if all(evidence.values()) else "mismatch"}
    write_json(work / "result.json", result)
    write_json(out / "result.json", result)
    return result


def positive(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("Must be positive")
    return number


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    plan = commands.add_parser("plan", help="pin the external inputs")
    plan.add_argument("--dt-commit", default="")
    plan.add_argument("--out", type=Path, required=True)
    run = commands.add_parser("run", help="run both compilers in one isolated container")
    run.add_argument("--manifest", type=Path, required=True)
    run.add_argument("--typescript", type=Path, required=True, help="pinned TypeScript checkout with built API/tsc")
    run.add_argument("--candidate-package", type=Path, required=True)
    run.add_argument("--candidate-platform", type=Path, required=True)
    run.add_argument("--out", type=Path, required=True)
    run.add_argument("--scratch", type=Path, required=True)
    run.add_argument("--sandbox-image", default="tsrs-dt-conformance")
    run.add_argument("--setup-timeout", type=positive, default=2700)
    run.add_argument("--check-timeout", type=positive, default=7200)
    run.add_argument("--processes", type=positive, default=min(os.cpu_count() or 1, 32))
    commands.add_parser("_suite", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.command == "plan":
        try:
            create_manifest(args.dt_commit, args.out)
        except ValueError as error:
            parser.error(str(error))
    elif args.command == "run":
        from sandbox import check_in_container

        manifest = json.loads(args.manifest.read_text())
        check_inputs(manifest)
        result = check_in_container(
            manifest, args.typescript, args.candidate_package, args.candidate_platform, args.out, args.scratch,
            args.setup_timeout, args.check_timeout, args.processes, args.sandbox_image,
        )
        return 0 if result["status"] == "match" else 1
    else:
        try:
            request = json.loads(Path("/request.json").read_text())
            if request.get("probe"):
                run_probe(request)
            else:
                run_suite(request)
        except BaseException as error:
            failed_suite(error)
    return 0


if __name__ == "__main__":
    sys.exit(main())
