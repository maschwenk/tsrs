# usage: TSGO=<reference tsgo> CAPS='<client caps json>' python3 drive.py <dir> reqs.json > expected.txt
import json, subprocess, sys, os
def reference_binary():
    # Same lookup as tools/oracle/emit/run.py: $TSGO, else $TSRS_WORK/bin/tsgo-ref.
    if os.environ.get('TSGO'):
        return os.environ['TSGO']
    work = os.environ.get('TSRS_WORK')
    if work and os.path.exists(os.path.join(work, 'bin', 'tsgo-ref')):
        return os.path.join(work, 'bin', 'tsgo-ref')
    sys.exit('set TSGO to the reference tsgo binary (or TSRS_WORK to a directory with bin/tsgo-ref)')
proj=os.path.abspath(sys.argv[1]); reqs=json.load(open(sys.argv[2]))
p=subprocess.Popen([reference_binary(),'--lsp','-stdio'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
def send(o):
    b=json.dumps(o).encode(); p.stdin.write(b'Content-Length: %d\r\n\r\n'%len(b)+b); p.stdin.flush()
def recv():
    h={}
    while True:
        l=p.stdout.readline().decode().strip()
        if not l: break
        k,v=l.split(':',1); h[k.lower()]=v.strip()
    return json.loads(p.stdout.read(int(h['content-length'])))
nid=[100]
def req(m,params):
    nid[0]+=1; i=nid[0]
    send({'jsonrpc':'2.0','id':i,'method':m,'params':params})
    while True:
        r=recv()
        if 'id' in r and 'method' in r:
            send({'jsonrpc':'2.0','id':r['id'],'result':None}); continue
        if r.get('id')==i: return r
base='file://'+proj+'/'
req('initialize',{'processId':None,'rootUri':'file://'+proj,'capabilities':json.loads(os.environ.get('CAPS','{}'))})
send({'jsonrpc':'2.0','method':'initialized','params':{}})
texts={}
for f in ['a.ts','b.ts']:
    texts[f]=open(proj+'/'+f).read()
    send({'jsonrpc':'2.0','method':'textDocument/didOpen','params':{'textDocument':{'uri':base+f,'languageId':'typescript','version':1,'text':texts[f]}}})
def pos(f,marker,off=0):
    t=texts[f]; o=t.find(marker)+off; line=t.count('\n',0,o); ch=o-(t.rfind('\n',0,o)+1)
    return {'line':line,'character':ch}
def fix(o): return json.loads(json.dumps(o).replace(base,'file:///').replace(proj+'/','/').replace(proj.lower()+'/','/'))
for r in reqs:
    m=r['m']; f=r['f']
    params={'textDocument':{'uri':base+f},'position':pos(f,r['at'],r.get('off',0))}
    params.update(json.loads(json.dumps(r.get('params',{})).replace('file:///',base)))
    if m.startswith('callHierarchy/'):
        items=req('textDocument/prepareCallHierarchy',params)['result']
        res=req(m,{'item':items[0]})
    else:
        res=req(m,params)
    out=res.get('result') if 'error' not in res else {'error':res['error']}
    print(json.dumps(fix(out),separators=(',',':')))
p.kill()
