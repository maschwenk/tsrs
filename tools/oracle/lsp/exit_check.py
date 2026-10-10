#!/usr/bin/env python3
"""Exit behavior check: `tsgo-ref --lsp -stdio` vs `tsrs --lsp -stdio`.

Four scenarios: with / without `shutdown` before `exit`, and with / without the client answering the server's
requests (`client/registerCapability` sent while handling `initialized`). Prints exit status and the stderr tail per
server and fails if the two servers differ. Both servers block their dispatch loop on the unanswered
registration (Go's `sendClientRequest` from a synchronous handler), so `exit` alone ends them only when the client
answers; then both exit with status 1 and "context canceled" (Go's `Run` returns the errgroup's error).
"""
import json, subprocess, sys, time, os, threading
REPO=os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)),"..","..",".."))
TSRS_WORK=os.environ.get("TSRS_WORK", os.path.abspath(os.path.join(REPO,"..","..")))
def frame(o):
    b=json.dumps(o).encode(); return b"Content-Length: %d\r\n\r\n"%len(b)+b
def run(cmd, shutdown, answer):
    p=subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    def reader():
        buf=b""
        while True:
            c=p.stdout.read(1)
            if not c: return
            buf+=c
            if buf.endswith(b"\r\n\r\n"):
                n=int(buf.split(b":")[1].split(b"\r")[0]); m=json.loads(p.stdout.read(n)); buf=b""
                if answer and "method" in m and "id" in m:
                    try: p.stdin.write(frame({"jsonrpc":"2.0","id":m["id"],"result":None})); p.stdin.flush()
                    except Exception: pass
    threading.Thread(target=reader,daemon=True).start()
    p.stdin.write(frame({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":None,"rootUri":None,"capabilities":{}}}))
    p.stdin.write(frame({"jsonrpc":"2.0","method":"initialized","params":{}})); p.stdin.flush(); time.sleep(0.5)
    if shutdown: p.stdin.write(frame({"jsonrpc":"2.0","id":2,"method":"shutdown"}))
    p.stdin.write(frame({"jsonrpc":"2.0","method":"exit"})); p.stdin.flush()
    t=time.time()
    try: rc=p.wait(timeout=10)
    except subprocess.TimeoutExpired: rc="TIMEOUT"; p.kill()
    err=p.stderr.read().decode()[-200:]
    return (rc, err.strip()[-60:])
results=[]
bad=0
for a in (True, False):
  for s in (True, False):
    ra=run([TSRS_WORK+"/bin/tsgo-ref","--lsp","-stdio"], s, a)
    rb=run([os.path.join(REPO,"target/release/tsrs"),"--lsp","-stdio"], s, a)
    ok = ra==rb
    bad += not ok
    print(f"{'shutdown' if s else 'no shutdown':<12} {'answers' if a else 'silent client':<14} tsgo-ref={ra} tsrs={rb} {'equal' if ok else 'DIFFERENT'}")
sys.exit(1 if bad else 0)
