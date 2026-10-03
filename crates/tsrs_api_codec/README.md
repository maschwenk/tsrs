# tsrs_api_codec

Binary AST codec of the TypeScript native API (`--api`): a Rust port of the pinned
`microsoft/TypeScript` `tsc/internal/api/encoder` package (commit in the workspace `Cargo.toml`,
`b85298b6a81f772d080b0455de0ca9d744cd6fd6`). It produces exactly the bytes the pinned Go server sends for
`getSourceFile` / `createSourceFile` / `typeToTypeNode` / …, which the pinned JavaScript client
(`packages/typescript/src/api/node`, `RemoteSourceFile`/`RemoteNode`) decodes lazily, and decodes the bytes the
client sends back (`printNode`, `formatNode`-style requests). It does not define a new format.

Derived from TypeScript (Apache-2.0, Copyright (c) Microsoft Corporation); see the repository `NOTICE`.

## API

| Rust | Pinned Go |
| --- | --- |
| `encode_source_file(&'static SourceFile) -> (Vec<u8>, NodeIndexTable)` | `EncodeSourceFile` |
| `encode_node(P<Node>, Option<&'static SourceFile>) -> (Vec<u8>, NodeIndexTable)` | `EncodeNode` |
| `build_node_index_table(&'static SourceFile) -> NodeIndexTable` | `BuildNodeIndexTable` |
| `NodeIndexTable::{node(i), get_index(n)}` | `NodeIndexTable.Nodes[i]`, `GetIndex` |
| `set_source_file_id`, `set_source_file_lease`, `source_file_hash` | `SetSourceFileID`, `SetSourceFileLease`, `SourceFileHash` |
| `decode_nodes(&[u8]) -> DecodedNode` (owns a fresh `Region`) | `DecodeNodes` |
| `decode_source_file(&[u8]) -> DecodedNode` | `DecodeSourceFile` |
| `decode_nodes_in_current_region(&[u8], &NodeFactory) -> P<Node>` | `DecodeNodes` into the caller's region |
| `PositionMap` (WTF-8 aware UTF-8 ⇄ UTF-16) | `ast.PositionMap` |

Node handles on the wire are `"<index>.<kind>.<path>"` (session.go `nodeHandleFrom`); `index` is the
`NodeIndexTable` index. The session owns building/caching the table per source file and must drop it together
with the snapshot/lease that keeps the file's region alive: the table holds raw `P<Node>` arena pointers and
the codec never extends their lifetime. `DecodedNode::root()` is valid only while the `DecodedNode` lives.

## Generation

`node crates/tsrs_api_codec/gen/gen-codec.ts` regenerates `src/generated.rs` and `COVERAGE.generated.md` from
`ts-ref/tools/scripts/tsc/ast.json` through `tools/gen-ast/schema.ts` (the same inputs as `tsrs_ast`), porting the
classification of the pinned `tools/scripts/tsc/generate-encoder.ts` (data type, children masks in visitor
order, 6-bit commonData layout incl. SyntaxKind-union indices, string/extended data, decoder factories with
kind-alias deconfliction). `--check` fails if the checked-in files are stale. The traversal is tsrs_ast's
port of Go's `NodeVisitor.VisitEachChild` with the encoder's `VisitNodes`/`VisitModifiers` hooks plus
`node.jsdoc(file)`, so record order, NodeList records, empty-modifier elision and the unmasked
`FullSignature` child match Go.

## Verification

* `tests/golden/*.bin` are produced by the pinned server itself (`gen/oracle.mjs`, typescript
  `7.1.0-dev.20260930.4` = pinned commit + one CI-only commit) for public fixtures (`tests/fixtures`: TS, TSX,
  `.d.ts`, checked JS with JSDoc, BOM/CRLF/astral identifiers, string escapes producing lone surrogates
  (WTF-8), parse errors, empty file, triple-slash references, module augmentations, imports/exports of every
  form). `golden_compat` requires byte equality of the whole encoding (header incl. xxh3 hash words and parse
  options, string table, extended and structured msgpack data, every node record).
* The decoder is tested on the Go server's bytes and on the JavaScript client encoder's bytes
  (`*.client.bin`), and against malformed input.
* `tests/node_client.mjs` decodes Rust-produced bytes with the real JavaScript client.

## Known gaps / divergences

* Content-mapped source files are not ported in tsrs (`is_content_mapped()` is always false), so the span map,
  supplemental/canonical file names, content mapper, virtual file name and diagnostic directive fields are
  always the "absent" values; those are what the pinned encoder writes for an ordinary file.
* Binder data (header offset 60) is always 0, as in the pinned encoder.
* `get_index` returns the lowest index for a node that occurs twice (reparsed JSDoc types); Go returns an
  arbitrary one of them. Both resolve to the same node.
* The decoder is stricter than Go's: out-of-range string/extended offsets, sibling links that do not point
  forward to a child of the same parent, unknown kinds/enum values, invalid UTF-8 (other than WTF-8 lone
  surrogates), missing required children and the reserved data type are errors instead of panics/zero values.
  Decoded positions are taken as `i32` (wire `0xFFFFFFFF` becomes the synthesized position -1; Go widens it to
  4294967295).
* Source text with unpaired surrogates cannot be produced through the pinned server (its JSON request decoding
  rejects it), so no golden covers lone surrogates in file text; `PositionMap` follows Go's
  `DecodeJSStringRune` for them.
