// Regenerates tests/golden/*.bin with the pinned TypeScript native API (the Go `tsc --api` server and its
// JavaScript client), the reference implementation this crate must match byte for byte.
//
// Usage:
//   npm install --prefix /tmp/tsoracle typescript@7.1.0-dev.20260930.4   # = pinned commit + 1 CI-only commit
//   TSRS_CODEC_ORACLE=/tmp/tsoracle/node_modules/typescript node crates/tsrs_api_codec/gen/oracle.mjs
//
// For every fixture it asks the server to `createSourceFile` (parse + bind through the server's parse cache,
// then `encoder.EncodeSourceFile`) and stores the response with the session-specific source file ID and lease
// fields (header bytes 44..60) zeroed. It also stores, for the decoder tests, the bytes the JavaScript client's
// own encoder (`encodeNode`/`encodeSourceFile`, used for printNode and friends) produces for each tree.
import * as fs from "node:fs";
import * as path from "node:path";
import { pathToFileURL } from "node:url";

const crateRoot = path.resolve(import.meta.dirname, "..");
const oracle = process.env.TSRS_CODEC_ORACLE;
if (!oracle) {
    console.error("set TSRS_CODEC_ORACLE to an installed typescript@7.1.0-dev.20260930.4 package directory");
    process.exit(2);
}
const pkg = JSON.parse(fs.readFileSync(path.join(oracle, "package.json"), "utf8"));
const imp = rel => import(pathToFileURL(path.join(oracle, rel)).href);
const { API } = await imp("dist/api/sync/api.js");
const { encodeSourceFile, encodeNode } = await imp("dist/api/node/encoder.js");

const fixturesDir = path.join(crateRoot, "tests/fixtures");
const goldenDir = path.join(crateRoot, "tests/golden");
fs.mkdirSync(goldenDir, { recursive: true });

// Source text with unpaired surrogates cannot be sent: the pinned server rejects it while unmarshalling the
// request ("jsontext: invalid surrogate pair"). Lone surrogates still reach the binary format through escapes in
// string literals (unicode_escapes.ts), whose cooked text is WTF-8.
const inline = {};

const fixtures = fs.readdirSync(fixturesDir).filter(f => !f.startsWith(".")).sort()
    .map(name => [name, fs.readFileSync(path.join(fixturesDir, name), "utf8")]);
fixtures.push(...Object.entries(inline));

const api = new API({ cwd: "/" });
const manifest = { oracle: `${pkg.name}@${pkg.version}`, gitHead: pkg.gitHead, files: {} };
for (const [name, text] of fixtures) {
    using retained = api.createSourceFile(`/fixtures/${name}`, text);
    const sf = retained.sourceFile ?? retained;
    const raw = api.client.apiRequestBinary("createSourceFile", { fileName: `/fixtures/${name}`, sourceText: text, options: {} });
    const lease = new DataView(raw.buffer, raw.byteOffset).getUint32(52, true);
    api.client.apiRequest("releaseSourceFile", { lease });
    const data = Uint8Array.from(raw);
    data.fill(0, 44, 60);
    fs.writeFileSync(path.join(goldenDir, `${name}.bin`), data);
    const client = Uint8Array.from(encodeSourceFile(sf));
    fs.writeFileSync(path.join(goldenDir, `${name}.client.bin`), client);
    manifest.files[name] = { bytes: data.length, clientBytes: client.length, inline: name in inline };
    console.log(`${name}: ${data.length} bytes (client encoder ${client.length})`);
}
api.close();
fs.writeFileSync(path.join(goldenDir, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
