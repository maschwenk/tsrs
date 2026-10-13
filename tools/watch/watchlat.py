# usage: watchlat.py <compiler> <projdir> [extra args...]  -> initial build and per-edit save-to-report latency
import subprocess, sys, time, os, threading, queue, signal
comp, proj, extra = sys.argv[1], sys.argv[2], sys.argv[3:]
REPEAT = int(os.environ.get("REPEAT", "1"))
EDITS = [
    ("body comment, leaf-ish", "vs/base/common/strings.ts", lambda s: s + "\n// watchbench edit\n"),
    ("body comment again",     "vs/base/common/strings.ts", lambda s: s + "\n// watchbench edit 2\n"),
    ("new export, hub file",   "vs/base/common/lifecycle.ts", lambda s: s + "\nexport const watchbenchX = 1;\n"),
    ("type error in leaf",     "vs/base/common/strings.ts", lambda s: s + "\nconst watchbenchErr: number = 'x';\n"),
]
MODE = ["-b", os.environ["BUILD"], "--watch"] if os.environ.get("BUILD") else ["--watch", "-p", "."]
p = subprocess.Popen([comp, *MODE, "--pretty", "false", *extra], cwd=proj, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
q = queue.Queue()
LOG = open(os.environ["LOG"], "w") if os.environ.get("LOG") else None
def pump():
    for l in p.stdout:
        if LOG: LOG.write(l); LOG.flush()
        q.put((time.monotonic(), l))
threading.Thread(target=pump, daemon=True).start()
def rss_mb():
    # ps works on Linux and macOS. On macOS also look at `footprint <pid>` (RSS misses compressed memory).
    r = subprocess.run(["ps", "-o", "rss=", "-p", str(p.pid)], capture_output=True, text=True)
    return int(r.stdout.strip()) // 1024 if r.stdout.strip() else -1
def footprint_mb():
    # macOS: the physical footprint, which counts compressed pages that RSS leaves out.
    if sys.platform != "darwin":
        return ""
    r = subprocess.run(["footprint", "-p", str(p.pid)], capture_output=True, text=True)
    for word in r.stdout.split("Footprint:")[1:2]:
        n, unit = word.split()[:2]
        return f"  footprint {float(n) * (1024 if unit == 'GB' else 1):.0f} MB"
    return ""
def wait_ready(t0, timeout=300):
    # Returns (first detection, last report, cycles) once no output has arrived for 3 s after a report.
    detected = None; last = None; cycles = 0; msg = ""
    while True:
        try:
            t, l = q.get(timeout=3 if last is not None else timeout)
        except queue.Empty:
            return detected, last, cycles, msg
        if "File change detected" in l:
            cycles += 1
            if detected is None: detected = t - t0
        if "Watching for file changes" in l:
            last = t - t0; msg = l.strip().split(" - ")[-1]
t0 = time.monotonic()
_, ready, _, msg = wait_ready(t0)
print(f"{os.path.basename(comp):10} initial  {ready*1000:8.0f} ms  rss {rss_mb()} MB{footprint_mb()}  {msg}")
originals = {}
try:
    for name, rel, f in EDITS * REPEAT:
        path = os.path.join(proj, rel)
        originals.setdefault(path, open(path).read())
        cur = open(path).read()
        t0 = time.monotonic()
        with open(path, "w") as fh: fh.write(f(cur))
        det, ready, cycles, msg = wait_ready(t0)
        print(f"{os.path.basename(comp):10} {name:24} detected {det*1000 if det else float('nan'):7.1f} ms  done {ready*1000 if ready else float('nan'):8.0f} ms  cycles {cycles}  rss {rss_mb()} MB{footprint_mb()}  {msg}")
finally:
    for path, s in originals.items(): open(path, "w").write(s)
    time.sleep(1.0)
    p.send_signal(signal.SIGINT)
    try: p.wait(timeout=10)
    except subprocess.TimeoutExpired: p.kill()
