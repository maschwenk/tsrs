#!/usr/bin/env node
// A content mapper for tsrs's end-to-end test: TypeScript 7.1 content-mapper protocol (JSON-RPC 2.0 over stdio
// with Content-Length framing). It turns every mapped file into `<synthesized header>\n<original text>`, so a
// diagnostic in the original text sits at a virtual position that differs from its original position and the
// compiler must map it back. A line `//!mapper-error` in the original reports a mapper-authored diagnostic.
import process from 'node:process'

const HEADER = '// synthesized by header-mapper\nexport {};\n'
const SOURCE = 'header'

function respond(message) {
  const body = Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...message }))
  process.stdout.write(`Content-Length: ${body.length}\r\n\r\n`)
  process.stdout.write(body)
}

function transform(params) {
  const content = params.content
  const text = HEADER + content
  const diagnostics = []
  const marker = '//!mapper-error'
  const index = content.indexOf(marker)
  if (index !== -1) {
    diagnostics.push({ messageText: 'The mapper rejected this line.', start: index, length: marker.length, code: 7 })
  }
  return {
    text,
    extension: '.ts',
    // [virtualStart, virtualLength, originalStart, originalLength, kind] with kind 0 = verbatim
    mappings: content.length ? [[HEADER.length, content.length, 0, content.length, 0]] : [],
    diagnostics,
  }
}

function handle(request) {
  if (request.id === undefined) return
  switch (request.method) {
    case 'initialize':
      respond({ id: request.id, result: { positionEncoding: 'utf-8', diagnosticSource: SOURCE } })
      break
    case 'openProject':
      respond({ id: request.id, result: {} })
      break
    case 'closeProject':
      respond({ id: request.id, result: null })
      break
    case 'transform':
      respond({ id: request.id, result: transform(request.params) })
      break
    default:
      respond({ id: request.id, error: { code: -32601, message: `Unknown method: ${request.method}` } })
  }
}

let pending = Buffer.alloc(0)
process.stdin.on('data', (chunk) => {
  pending = Buffer.concat([pending, chunk])
  for (;;) {
    const headerEnd = pending.indexOf('\r\n\r\n')
    if (headerEnd === -1) return
    const match = /Content-Length:\s*(\d+)/i.exec(pending.subarray(0, headerEnd).toString())
    if (!match) throw new Error('missing Content-Length')
    const length = Number(match[1])
    const start = headerEnd + 4
    if (pending.length < start + length) return
    const body = pending.subarray(start, start + length).toString()
    pending = pending.subarray(start + length)
    handle(JSON.parse(body))
  }
})
process.stdin.on('end', () => process.exit(0))
