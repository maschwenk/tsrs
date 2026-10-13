# usage: watchcheck.py <compiler> <projdir>
# Applies a sequence of edits under a running `--watch`, and after each cycle compares the diagnostics the watch
# printed with a fresh `--noEmit` build of the same tree. Restores every touched file at the end.
# BUILD=<config> runs `-b <config> --watch` instead (the fresh build uses `-p <config>`).
import os, queue, signal, subprocess, sys, threading, time

comp, proj = sys.argv[1], sys.argv[2]
STRINGS = "vs/base/common/strings.ts"
LIFECYCLE = "vs/base/common/lifecycle.ts"
NEW = "vs/base/common/watchcheckNew.ts"
CONSUMER = "vs/base/common/arrays.ts"

def append(text):
    return lambda s: s + text

def drop(text):
    return lambda s: s.replace(text, "")

EDITS = [
    ("comment", STRINGS, append("\n// watchcheck comment\n")),
    ("type error", STRINGS, append("\nconst watchcheckErr: number = 'x';\n")),
    ("fix type error", STRINGS, drop("\nconst watchcheckErr: number = 'x';\n")),
    ("new export in hub", LIFECYCLE, append("\nexport const watchcheckX = 1;\n")),
    ("new file using it", NEW, lambda s: "import { watchcheckX } from './lifecycle';\nexport const y: string = watchcheckX;\n"),
    ("fix new file", NEW, lambda s: "import { watchcheckX } from './lifecycle';\nexport const y: number = watchcheckX;\n"),
    ("remove the export", LIFECYCLE, drop("\nexport const watchcheckX = 1;\n")),
    ("delete new file", NEW, None),
    ("break an import", CONSUMER, lambda s: "import { doesNotExist } from './strings';\n" + s),
    ("unbreak the import", CONSUMER, lambda s: s.replace("import { doesNotExist } from './strings';\n", "", 1)),
    ("export type change", STRINGS, lambda s: s.replace("export function isFalsyOrWhitespace(str: string | undefined): boolean {", "export function isFalsyOrWhitespace(str: string | undefined): number {", 1)),
    ("export type restore", STRINGS, lambda s: s.replace("export function isFalsyOrWhitespace(str: string | undefined): number {", "export function isFalsyOrWhitespace(str: string | undefined): boolean {", 1)),
]

BUILD = os.environ.get("BUILD")
MODE = ["-b", BUILD, "--watch"] if BUILD else ["--watch", "-p", "."]
p = subprocess.Popen([comp, *MODE, "--pretty", "false"], cwd=proj, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
q = queue.Queue()
threading.Thread(target=lambda: [q.put(l) for l in p.stdout], daemon=True).start()

def next_cycle(timeout=300):
    # The diagnostics of the next completed cycle (lines between its start and its "Found N errors" line), once
    # nothing else has arrived for 3 s.
    lines, cycles, last = [], [], None
    while True:
        try:
            l = q.get(timeout=3 if last is not None else timeout)
        except queue.Empty:
            return cycles
        if "Starting compilation" in l or "File change detected" in l:
            lines = []
            continue
        if "Watching for file changes" in l:
            cycles.append((lines, l.strip().split(" - ")[-1]))
            last = l
            continue
        lines.append(l.rstrip("\n").replace("\x1b[2J\x1b[3J\x1b[H", ""))

def fresh():
    r = subprocess.run([comp, "-p", BUILD or ".", "--pretty", "false", "--noEmit"], cwd=proj, capture_output=True, text=True)
    return [l for l in r.stdout.split("\n")]

def normalize(lines):
    return [l for l in lines if l.strip()]

originals = {}
ok = True
try:
    cycles = next_cycle()
    for name, rel, f in [("initial", None, None)] + EDITS:
        if rel is not None:
            path = os.path.join(proj, rel)
            if path not in originals:
                originals[path] = open(path).read() if os.path.exists(path) else None
            if f is None:
                os.remove(path)
            else:
                cur = open(path).read() if os.path.exists(path) else ""
                with open(path, "w") as fh:
                    fh.write(f(cur))
            cycles = next_cycle()
        if not cycles:
            print(f"{name:22} NO CYCLE"); ok = False; continue
        watch_lines, summary = cycles[-1]
        want = normalize(fresh())
        got = normalize(watch_lines)
        same = got == want
        ok &= same
        print(f"{name:22} cycles {len(cycles)}  {summary:45} {'SAME' if same else 'DIFF'}  ({len(got)} vs {len(want)} lines)")
        if not same:
            gs, ws = set(got), set(want)
            for l in sorted(gs - ws)[:5]: print("   watch only:", l[:160])
            for l in sorted(ws - gs)[:5]: print("   fresh only:", l[:160])
    if os.environ.get("EMITCHECK"):
        # Stop the watch on the edited tree, then compare what it emitted with a fresh emit of that tree.
        p.send_signal(signal.SIGINT)
        p.wait(timeout=10)
        outdir = os.environ["EMITCHECK"]
        fresh_out = outdir + "-fresh"
        subprocess.run(["rm", "-rf", fresh_out])
        subprocess.run([comp, "-p", ".", "--pretty", "false", "--outDir", fresh_out], cwd=proj, capture_output=True)
        r = subprocess.run(["diff", "-rq", outdir, fresh_out], capture_output=True, text=True)
        n = sum(len(fs) for _, _, fs in os.walk(fresh_out))
        print(f"emit check: {n} fresh files; {'IDENTICAL' if r.returncode == 0 else 'DIFFERENT'}")
        print("\n".join(r.stdout.split("\n")[:10]))
        ok &= r.returncode == 0
finally:
    for path, s in originals.items():
        if s is None:
            if os.path.exists(path): os.remove(path)
        else:
            open(path, "w").write(s)
    time.sleep(1.0)
    p.send_signal(signal.SIGINT)
    try: p.wait(timeout=10)
    except subprocess.TimeoutExpired: p.kill()
print("ALL SAME" if ok else "MISMATCH")
