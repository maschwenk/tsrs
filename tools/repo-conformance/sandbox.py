"""Keep repository-controlled code out of the runner's filesystem, processes and caches."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import uuid


def launch(image: str, worker: Path, tsgo: Path, tsrs: Path, work: Path, request: Path, log: Path,
           timeout: float, source: Path | None = None) -> dict:
    """Run one repository, then remove its container and all descendants before returning or reading outputs."""
    if os.getuid() == 0:
        raise ValueError("Run repository conformance as a non-root host user")
    name = "tsrs-conformance-" + uuid.uuid4().hex
    mounts = [(worker, "/harness/tools/repo-conformance/run.py", True), (tsgo, "/inputs/tsgo", True),
              (tsrs, "/inputs/tsrs", True), (request, "/request.json", True), (work, "/work", False)]
    if source:
        mounts.append((source, "/source", True))
    command = ["docker", "run", "--pull=never", "--name", name, "--label", "tsrs.repo-conformance",
               "--init", "--read-only", "--user", f"{os.getuid()}:{os.getgid()}", "--cap-drop=ALL",
               "--security-opt=no-new-privileges", "--pids-limit=512", "--network=bridge", "--workdir=/work",
               "--env=HOME=/work/home", "--env=TMPDIR=/work/tmp", "--entrypoint=python3"]
    for path, target, readonly in mounts:
        command += ["--mount", f"type=bind,source={path.resolve()},target={target}" + (",readonly" if readonly else "")]
    command += [image, "-B", "/harness/tools/repo-conformance/run.py", "_repo"]
    (work / "tmp").mkdir(parents=True)
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w") as output:
        try:
            process = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, timeout=timeout)
            result = {"exit": process.returncode, "timeout": False}
        except subprocess.TimeoutExpired:
            result = {"exit": -1, "timeout": True}
        finally:
            # Removing the whole PID namespace also kills detached/setsid children after a successful install.
            cleanup = subprocess.run(["docker", "rm", "--force", name], stdout=output, stderr=subprocess.STDOUT,
                                     timeout=30)
            if cleanup.returncode:
                raise RuntimeError(f"Could not remove sandbox {name}; refusing to start another repository")
    return result


def copy_logs(source: Path, target: Path) -> None:
    """Container outputs are untrusted: never follow a link back into the host when exporting artifacts."""
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


def check_in_container(repo: dict, tsgo: Path, tsrs: Path, out: Path, scratch: Path, setup_timeout: int,
                       check_timeout: int, max_configs: int, image: str, source: Path | None = None) -> dict:
    scratch.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="repository-", dir=scratch) as temporary:
        base = Path(temporary)
        work = base / "work"
        request = base / "request.json"
        request.write_text(json.dumps(dict(repo=repo, setup_timeout=setup_timeout, check_timeout=check_timeout,
                                           max_configs=max_configs, source=bool(source))))
        ran = launch(image, Path(__file__).with_name("run.py"), tsgo, tsrs, work, request, out / "sandbox.log",
                     setup_timeout + check_timeout + 30, source)
        copy_logs(work / "out", out)
        if ran["exit"] or ran["timeout"]:
            return dict(repo, status="setup_failed", configs=[], reason="Sandbox failed or timed out; see sandbox.log")
        result = work / "result.json"
        if not stat.S_ISREG(result.lstat().st_mode) or result.stat().st_size > 1024 * 1024:
            raise ValueError("Invalid sandbox result file")
        data = json.loads(result.read_text())
        statuses = {"setup_failed", "no_config", "mismatch", "tsrs_failed", "match", "partial", "not_checked"}
        if data.get("repository") != repo["repository"] or data.get("status") not in statuses:
            raise ValueError("Invalid sandbox repository result")
        return data
