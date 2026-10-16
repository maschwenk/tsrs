"""Run the complete DefinitelyTyped suite without exposing the CI runner."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import uuid


def launch(image: str, worker: Path, typescript: Path, candidate_package: Path, candidate_platform: Path,
           work: Path, request: Path, log: Path, timeout: float) -> dict:
    if os.getuid() == 0:
        raise ValueError("Run DefinitelyTyped conformance as a non-root host user")
    name = "tsrs-dt-conformance-" + uuid.uuid4().hex
    mounts = [
        (worker, "/harness/tools/dt-conformance/run.py", True),
        (typescript, "/inputs/typescript", True),
        (candidate_package, "/inputs/tsrs-package", True),
        (candidate_platform, "/inputs/tsrs-platform", True),
        (request, "/request.json", True),
        (work, "/work", False),
    ]
    command = [
        "docker", "run", "--pull=never", "--name", name, "--label", "tsrs.dt-conformance", "--init",
        "--read-only", "--user", f"{os.getuid()}:{os.getgid()}", "--cap-drop=ALL",
        "--security-opt=no-new-privileges", "--pids-limit=4096", "--network=bridge", "--workdir=/work",
        "--env=HOME=/work/home", "--env=TMPDIR=/work/tmp", "--entrypoint=python3",
    ]
    for path, target, readonly in mounts:
        command += ["--mount", f"type=bind,source={path.resolve()},target={target}" + (",readonly" if readonly else "")]
    command += [image, "-B", "/harness/tools/dt-conformance/run.py", "_suite"]
    (work / "tmp").mkdir(parents=True)
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w") as output:
        try:
            process = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, timeout=timeout)
            result = {"exit": process.returncode, "timeout": False}
        except subprocess.TimeoutExpired:
            result = {"exit": -1, "timeout": True}
        finally:
            cleanup = subprocess.run(["docker", "rm", "--force", name], stdout=output, stderr=subprocess.STDOUT,
                                     timeout=30)
            if cleanup.returncode:
                raise RuntimeError(f"Could not remove sandbox {name}")
    return result


def copy_outputs(source: Path, target: Path) -> None:
    """Export only regular files; container output is untrusted."""
    if source.is_symlink() or not source.is_dir():
        return
    for directory, dirs, files in os.walk(source, followlinks=False):
        root = Path(directory)
        dirs[:] = [name for name in dirs if not (root / name).is_symlink()]
        destination = target / root.relative_to(source)
        destination.mkdir(parents=True, exist_ok=True)
        for name in files:
            path = root / name
            if stat.S_ISREG(path.lstat().st_mode):
                shutil.copyfile(path, destination / name)


def check_in_container(manifest: dict, typescript: Path, candidate_package: Path, candidate_platform: Path,
                       out: Path, scratch: Path, setup_timeout: int, check_timeout: int, processes: int,
                       image: str) -> dict:
    scratch.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="definitely-typed-", dir=scratch) as temporary:
        base = Path(temporary)
        work = base / "work"
        request = base / "request.json"
        request.write_text(json.dumps({
            "manifest": manifest,
            "setup_timeout": setup_timeout,
            "check_timeout": check_timeout,
            "processes": processes,
        }))
        ran = launch(image, Path(__file__).with_name("run.py"), typescript, candidate_package,
                     candidate_platform, work, request, out / "sandbox.log",
                     setup_timeout + check_timeout * 2 + 120)
        copy_outputs(work / "out", out)
        result_path = work / "result.json"
        if ran["exit"] or ran["timeout"] or not result_path.is_file() or result_path.is_symlink():
            result = {"status": "setup_failed", "reason": "Sandbox failed or timed out; see sandbox.log"}
            (out / "report.md").write_text("# DefinitelyTyped conformance\n\n**Setup Failed**\n\n" +
                                           result["reason"] + "\n")
            return result
        if result_path.stat().st_size > 4 * 1024 * 1024:
            raise ValueError("Invalid sandbox result file")
        result = json.loads(result_path.read_text())
        if result.get("status") not in {"match", "mismatch", "setup_failed"}:
            raise ValueError("Invalid sandbox result status")
        return result
