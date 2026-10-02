# Records completion-related responses from tsgo-ref --lsp -stdio for the tsrs_ls completions smoke test.
# usage: drive.py <proj> <cases.json>  -> prints one JSON object per case: {"id":..., "result":...}
import json, subprocess, sys, os
proj = os.path.abspath(sys.argv[1]); spec = json.load(open(sys.argv[2]))
p = subprocess.Popen(['/Users/maxschwenk/Developer/tsrs-work/bin/tsgo-ref', '--lsp', '-stdio'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
def send(o):
    b = json.dumps(o).encode(); p.stdin.write(b'Content-Length: %d\r\n\r\n' % len(b) + b); p.stdin.flush()
def recv():
    h = {}
    while True:
        l = p.stdout.readline().decode().strip()
        if not l: break
        k, v = l.split(':', 1); h[k.lower()] = v.strip()
    return json.loads(p.stdout.read(int(h['content-length'])))
def req(i, m, params):
    send({'jsonrpc': '2.0', 'id': i, 'method': m, 'params': params})
    while True:
        r = recv()
        if 'id' in r and 'method' in r:
            send({'jsonrpc': '2.0', 'id': r['id'], 'result': None}); continue
        if r.get('id') == i: return r
req(1, 'initialize', {'processId': None, 'rootUri': 'file://' + proj, 'capabilities': spec['capabilities'],
                      'initializationOptions': {'userPreferences': spec['preferences']}})
send({'jsonrpc': '2.0', 'method': 'initialized', 'params': {}})
texts = {}
for f in spec['files']:
    texts[f] = open(os.path.join(proj, f)).read()
    lang = {'.ts': 'typescript', '.tsx': 'typescriptreact', '.js': 'javascript'}[os.path.splitext(f)[1]]
    send({'jsonrpc': '2.0', 'method': 'textDocument/didOpen', 'params': {'textDocument': {'uri': 'file://' + proj + '/' + f, 'languageId': lang, 'version': 1, 'text': texts[f]}}})
results = {}
for n, c in enumerate(spec['cases']):
    if c['method'] == 'completionItem/resolve':
        prev = results[c['from']]
        item = next(i for i in prev['items'] if i['label'] == c['label'])
        r = req(100 + n, c['method'], item)
    else:
        text = texts[c['file']]; caret = c['caret']; needle = caret.replace('|', '')
        off = text.find(needle); assert off >= 0 and text.find(needle, off + 1) < 0, caret
        off += caret.index('|')
        line = text.count('\n', 0, off); ch = off - (text.rfind('\n', 0, off) + 1)
        params = {'textDocument': {'uri': 'file://' + proj + '/' + c['file']}, 'position': {'line': line, 'character': ch}}
        if c['method'] == 'textDocument/completion':
            params['context'] = c.get('context', {'triggerKind': 1})
        if c['method'] == 'textDocument/_vs_onAutoInsert':
            params = {'_vs_textDocument': params['textDocument'], '_vs_position': params['position'], '_vs_ch': c['ch']}
        r = req(100 + n, c['method'], params)
    res = r.get('result')
    results[c['id']] = res
    out = json.dumps({'id': c['id'], 'result': res, 'error': r.get('error')}, separators=(',', ':'), ensure_ascii=False)
    print(out.replace(proj, ''))
p.kill()
