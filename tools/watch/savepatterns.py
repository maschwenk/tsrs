# usage: savepatterns.py <compiler> <projdir> <file> [<many-dir>]
# Saves <file> (relative to <projdir>) under a running `--watch` in the ways editors and tools do, and for each
# pattern reports how many cycles the save started and whether the last cycle's diagnostics match a fresh
# `--noEmit` build. Every save adds a type error (so a cycle that read a half-written file reports something else),
# and the next save of the same pattern removes it. With <many-dir>, also rewrites up to 200 .ts files under it at
# once (a formatter) and, if <projdir> is in a git work tree, checks out a branch that touches them (`git checkout`).
# Restores every touched file at the end. BUILD=<config> runs `-b <config> --watch` instead. ONLY=<text> runs only the
# patterns whose name contains it. LOG=<file> saves the watch's output.
import os, queue, subprocess, sys, tempfile, threading, time

comp, proj, target = sys.argv[1], sys.argv[2], sys.argv[3]
many_dir = sys.argv[4] if len(sys.argv) > 4 else None
build = os.environ.get("BUILD")
ERR = "\nconst savepatternsErr: number = 'savepatterns half-written';\n"


def read(rel):
    with open(os.path.join(proj, rel)) as f:
        return f.read()


def inplace(rel, text):
    with open(os.path.join(proj, rel), "w") as f:
        f.write(text)


def slow_writer(rel, text):
    # Truncate, pause, then write: the watch sees an empty file first.
    with open(os.path.join(proj, rel), "w") as f:
        f.flush()
        time.sleep(0.2)
        f.write(text)


def chunked(rel, text):
    # Four writes over 300 ms, each flushed.
    with open(os.path.join(proj, rel), "w") as f:
        n = len(text) // 4 + 1
        for i in range(4):
            f.write(text[i * n:(i + 1) * n])
            f.flush()
            if i < 3:
                time.sleep(0.1)


def atomic(rel, text):
    path = os.path.join(proj, rel)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        f.write(text)
    os.rename(tmp, path)


def atomic_backup(rel, text):
    # vim's default (backupcopy=auto on a plain file): rename the original away, write a new file, delete the backup.
    path = os.path.join(proj, rel)
    os.rename(path, path + "~")
    with open(path, "w") as f:
        f.write(text)
    os.remove(path + "~")


PATTERNS = [("in-place write", inplace), ("truncate, 200 ms, write", slow_writer), ("4 chunked writes over 300 ms", chunked), ("atomic rename", atomic), ("rename away, write, delete backup", atomic_backup)]

args = ["-b", build, "--watch"] if build else ["--watch", "-p", "."]
p = subprocess.Popen([comp, *args, "--pretty", "false"], cwd=proj, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
q = queue.Queue()
LOG = open(os.environ["LOG"], "w") if os.environ.get("LOG") else None


def pump():
    for l in p.stdout:
        if LOG:
            LOG.write(f"{time.monotonic():.3f} {l}")
            LOG.flush()
        q.put(l)


threading.Thread(target=pump, daemon=True).start()


def next_cycles(quiet=3.0, timeout=300):
    # Every cycle that completes until nothing has arrived for `quiet` seconds after the first one.
    # A cycle that has started is waited for, however long it takes.
    lines, cycles, running = [], [], False
    while True:
        try:
            l = q.get(timeout=quiet if cycles and not running else timeout)
        except queue.Empty:
            return cycles
        if "Starting compilation" in l or "File change detected" in l or "Projects in this build" in l:
            lines, running = [], True
            continue
        if "Watching for file changes" in l:
            cycles.append(lines)
            lines, running = [], False
            continue
        lines.append(l.rstrip("\n").replace("\x1b[2J\x1b[3J\x1b[H", ""))


def fresh():
    a = ["-b", build, "--verbose", "false"] if build else ["-p", ".", "--noEmit"]
    r = subprocess.run([comp, *a, "--pretty", "false"], cwd=proj, capture_output=True, text=True)
    return r.stdout.split("\n")


def diags(lines):
    return sorted(l for l in lines if ": error TS" in l)


originals = {}


def remember(rel):
    if rel not in originals:
        originals[rel] = read(rel)


results = []


def check(name, act):
    act()
    cycles = next_cycles()
    got, want = (diags(cycles[-1]) if cycles else []), diags(fresh())
    same = bool(cycles) and got == want
    results.append((name, len(cycles), same))
    print(f"{name:40s} cycles {len(cycles)}  {'SAME' if same else 'DIFF'}", flush=True)
    if not same:
        for l in sorted(set(got) - set(want))[:5]:
            print("   watch only:", l[:200])
        for l in sorted(set(want) - set(got))[:5]:
            print("   fresh only:", l[:200])


try:
    remember(target)
    next_cycles()  # initial build
    base = originals[target]
    only = os.environ.get("ONLY")
    for name, save in PATTERNS:
        if only and only not in name:
            continue
        check(name + " (add error)", lambda: save(target, base + ERR))
        check(name + " (remove it)", lambda: save(target, base))
    if not only or only in "case-only rename":
        # On a case-insensitive volume FSEvents reports the new spelling; the program may hold the old one.
        d, n = os.path.split(target)
        swapped = os.path.join(d, n[0].swapcase() + n[1:])
        check("case-only rename", lambda: os.rename(os.path.join(proj, target), os.path.join(proj, swapped)))
        check("case-only rename back", lambda: os.rename(os.path.join(proj, swapped), os.path.join(proj, target)))
    if many_dir and not only:
        files = []
        for root, _, names in os.walk(os.path.join(proj, many_dir)):
            files.extend(os.path.relpath(os.path.join(root, n), proj) for n in sorted(names) if n.endswith(".ts") and not n.endswith(".d.ts"))
        files = sorted(files)[:200]
        for f in files:
            remember(f)
        check(f"formatter rewrites {len(files)} files", lambda: [inplace(f, originals[f] + "\n// savepatterns formatter\n") for f in files])
        check(f"formatter restores {len(files)} files", lambda: [inplace(f, originals[f]) for f in files])
        if subprocess.run(["git", "rev-parse"], cwd=proj, capture_output=True).returncode == 0:
            g = lambda *a: subprocess.run(["git", *a], cwd=proj, check=True, capture_output=True)
            head = subprocess.run(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=proj, capture_output=True, text=True).stdout.strip()
            if head == "HEAD":
                head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=proj, capture_output=True, text=True).stdout.strip()
            # The branch is made off the work tree's HEAD without touching the files (plumbing), then checked out.
            env = dict(os.environ, GIT_INDEX_FILE=os.path.join(tempfile.mkdtemp(), "index"))
            top = subprocess.run(["git", "rev-parse", "--show-toplevel"], cwd=proj, capture_output=True, text=True).stdout.strip()
            subprocess.run(["git", "read-tree", "HEAD"], cwd=proj, env=env, check=True)
            for f in files:
                blob = subprocess.run(["git", "hash-object", "-w", "--stdin"], cwd=proj, input=originals[f] + "\n// savepatterns branch\n", capture_output=True, text=True, check=True).stdout.strip()
                rel = os.path.relpath(os.path.join(proj, f), top)
                subprocess.run(["git", "update-index", "--cacheinfo", f"100644,{blob},{rel}"], cwd=proj, env=env, check=True)
            tree = subprocess.run(["git", "write-tree"], cwd=proj, env=env, capture_output=True, text=True, check=True).stdout.strip()
            commit = subprocess.run(["git", "commit-tree", tree, "-p", "HEAD", "-m", "savepatterns"], cwd=proj, capture_output=True, text=True, check=True).stdout.strip()
            os.remove(env["GIT_INDEX_FILE"])
            g("branch", "-f", "savepatterns", commit)
            check(f"git checkout touching {len(files)} files", lambda: g("checkout", "--quiet", "savepatterns"))
            check(f"git checkout back", lambda: g("checkout", "--quiet", head))
            g("branch", "-D", "savepatterns")
finally:
    for rel, text in originals.items():
        if read(rel) != text:
            inplace(rel, text)
    for leftover in [target + ".tmp", target + "~"]:
        if os.path.exists(os.path.join(proj, leftover)):
            os.remove(os.path.join(proj, leftover))
    p.terminate()
    p.wait()

bad = [r for r in results if r[1] != 1 or not r[2]]
print("ALL ONE CYCLE, ALL SAME" if not bad else f"{len(bad)} BAD: " + ", ".join(r[0] for r in bad))
