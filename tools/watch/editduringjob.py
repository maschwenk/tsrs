# usage: editduringjob.py <compiler> <projdir> [rounds]
# Edits right after each report (0-1.2 s, while the background signature job runs), alternating edits that keep and
# that change strings.ts's declarations. Prints the detection latency (it includes waiting for the cancelled job) and
# compares the diagnostics of each cycle with a fresh --noEmit build.
# BUILD=<config> runs `-b <config> --watch` instead (the fresh build uses `-p <config>`).
import os, queue, random, signal, subprocess, sys, threading, time

comp, proj = sys.argv[1], sys.argv[2]
rounds = int(sys.argv[3]) if len(sys.argv) > 3 else 8
path = os.path.join(proj, "vs/base/common/strings.ts")
orig = open(path).read()
SIG_OLD = "export function isFalsyOrWhitespace(str: string | undefined): boolean {"
SIG_NEW = "export function isFalsyOrWhitespace(str: string | undefined): number {"
assert SIG_OLD in orig
random.seed(7)
BUILD = os.environ.get("BUILD")
MODE = ["-b", BUILD, "--watch"] if BUILD else ["--watch", "-p", "."]

p = subprocess.Popen([comp, *MODE, "--pretty", "false"], cwd=proj, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
q = queue.Queue()
threading.Thread(target=lambda: [q.put((time.monotonic(), l)) for l in p.stdout], daemon=True).start()

def cycle():
    lines, detected = [], None
    while True:
        t, l = q.get(timeout=300)
        if "File change detected" in l or "Starting compilation" in l:
            detected, lines = t, []
            continue
        if "Watching for file changes" in l:
            return detected, t, lines
        lines.append(l.rstrip("\n"))

def fresh():
    r = subprocess.run([comp, "-p", BUILD or ".", "--pretty", "false", "--noEmit"], cwd=proj, capture_output=True, text=True)
    return sorted(l for l in r.stdout.split("\n") if l.strip())

ok = True
try:
    cycle()
    text = orig
    for i in range(rounds):
        time.sleep(random.uniform(0, 1.2))
        if i % 3 == 2:
            text = text.replace(SIG_OLD, SIG_NEW) if SIG_OLD in text else text.replace(SIG_NEW, SIG_OLD)
            kind = "declaration change"
        else:
            text = text + f"\n// edit {i}\n"
            kind = "comment"
        t0 = time.monotonic()
        open(path, "w").write(text)
        detected, done, lines = cycle()
        got = sorted(l for l in lines if l.strip())
        same = got == fresh()
        ok &= same
        print(f"{kind:20} detected {(detected - t0) * 1000:7.1f} ms  done {(done - t0) * 1000:7.0f} ms  {'SAME' if same else 'DIFF'}")
finally:
    open(path, "w").write(orig)
    time.sleep(1.0)
    p.send_signal(signal.SIGINT)
    try: p.wait(timeout=10)
    except subprocess.TimeoutExpired: p.kill()
print("ALL SAME" if ok else "MISMATCH")
