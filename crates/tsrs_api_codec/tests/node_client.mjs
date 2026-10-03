// Decodes Rust-produced encodings with the real pinned JavaScript client (RemoteSourceFile / decodeNode from
// typescript@7.1.0-dev.20260930.4) and compares everything the client exposes with the same view of the Go
// server's encoding of the same file. Driven by tests/node_client.rs.
//
// Usage: node node_client.mjs <typescript package dir> <rust output dir> <golden dir>
import * as fs from "node:fs";
import * as path from "node:path";
import { pathToFileURL } from "node:url";

const [oracle, rustDir, goldenDir] = process.argv.slice(2);
const imp = rel => import(pathToFileURL(path.join(oracle, rel)).href);
const { RemoteSourceFile, decodeNode } = await imp("dist/api/node/node.js");
const { Wtf8Decoder } = await imp("dist/api/node/wtf8.js");

function view(node, depth, out) {
    const line = [" ".repeat(depth) + node.kind, node.pos, node.end, node.flags];
    for (const key of ["text", "rawText", "escapedText", "operator", "token", "keyword", "isTypeOnly", "multiLine", "templateFlags", "tokenFlags", "phaseModifier", "isExportEquals", "isTypeOf", "isArrayType", "isBracketed", "isNameFirst", "containsOnlyTriviaWhiteSpaces"]) {
        let v;
        try { v = node[key]; } catch (e) { v = `throws:${e.message}`; }
        if (v !== undefined && typeof v !== "object" && typeof v !== "function") line.push(`${key}=${JSON.stringify(v)}`);
    }
    out.push(line.join(" "));
    node.forEachChild(child => void view(child, depth + 1, out), nodes => {
        out.push(" ".repeat(depth + 1) + `[${nodes.length} pos=${nodes.pos} end=${nodes.end} trailing=${nodes.hasTrailingComma}]`);
        for (const n of nodes) view(n, depth + 2, out);
    });
}

function describeSourceFile(bytes) {
    const sf = new RemoteSourceFile(bytes, new Wtf8Decoder());
    const out = [];
    const refs = r => r.map(x => `${x.pos}-${x.end}:${x.fileName}:${x.resolutionMode ?? ""}:${x.preserve ?? ""}`).join(",");
    out.push(`fileName=${sf.fileName} path=${sf.path} lv=${sf.languageVariant} sk=${sf.scriptKind} decl=${sf.isDeclarationFile}`);
    out.push(`text=${JSON.stringify(sf.text)}`);
    out.push(`refs=${refs(sf.referencedFiles)} types=${refs(sf.typeReferenceDirectives)} libs=${refs(sf.libReferenceDirectives)}`);
    out.push(`imports=${sf.imports.map(n => `${n.kind}@${n.pos}:${n.text}`).join(",")}`);
    out.push(`augmentations=${sf.moduleAugmentations.map(n => `${n.kind}@${n.pos}`).join(",")} ambient=${sf.ambientModuleNames.join(",")}`);
    out.push(`externalModuleIndicator=${sf.externalModuleIndicator === undefined ? "none" : sf.externalModuleIndicator === sf ? "self" : `${sf.externalModuleIndicator.kind}@${sf.externalModuleIndicator.pos}`}`);
    view(sf, 0, out);
    return out;
}

let files = 0, lines = 0, failures = 0;
for (const entry of fs.readdirSync(rustDir).filter(f => f.endsWith(".rust.bin")).sort()) {
    const name = entry.slice(0, -".rust.bin".length);
    const rust = new Uint8Array(fs.readFileSync(path.join(rustDir, entry)));
    if (name.startsWith("print_")) {
        const orig = new Uint8Array(fs.readFileSync(path.join(rustDir, `${name}.orig.bin`)));
        const a = [], b = [];
        view(decodeNode(orig), 0, a);
        view(decodeNode(rust), 0, b);
        files++;
        lines += b.length;
        const i = a.findIndex((l, k) => l !== b[k]);
        if (i >= 0 || a.length !== b.length) {
            failures++;
            console.error(`MISMATCH ${name} at line ${i}:\n  go re-enc:   ${a[i]}\n  rust re-enc:  ${b[i]}`);
        }
        continue;
    }
    if (name.startsWith("synthetic.")) {
        const expected = fs.readFileSync(path.join(rustDir, `${name}.expected.txt`), "utf8").trimEnd().split("\n");
        const actual = [];
        const walk = (n, d) => {
            const t = [8, 9, 10, 14, 15, 16, 17, 79, 80].includes(n.kind) ? ` text=${JSON.stringify(n.text)}` : "";
            actual.push(`${" ".repeat(d)}${n.kind} ${n.pos} ${n.end}${t}`);
            n.forEachChild(c => void walk(c, d + 1));
        };
        walk(decodeNode(rust), 0);
        const exp = expected;
        if (JSON.stringify(actual) !== JSON.stringify(exp)) {
            failures++;
            console.error(`MISMATCH ${name}\nexpected:\n${exp.join("\n")}\nactual:\n${actual.join("\n")}`);
        }
        files++;
        lines += actual.length;
        continue;
    }
    const go = new Uint8Array(fs.readFileSync(path.join(goldenDir, `${name}.bin`)));
    const a = describeSourceFile(go), b = describeSourceFile(rust);
    files++;
    lines += b.length;
    const i = a.findIndex((l, k) => l !== b[k]);
    if (i >= 0 || a.length !== b.length) {
        failures++;
        console.error(`MISMATCH ${name} at line ${i}:\n  go:   ${a[i]}\n  rust: ${b[i]}`);
    }
}
// With the pinned server available, its printNode of the Rust-encoded synthesized trees must equal what
// decode_nodes + tsrs_printer print for the same bytes.
let printed = 0;
if (process.env.TSRS_CODEC_TSC) {
    const { API } = await imp("dist/api/sync/api.js");
    const api = new API({ cwd: "/", tsserverPath: process.env.TSRS_CODEC_TSC });
    for (const entry of fs.readdirSync(rustDir).filter(f => f.startsWith("synthetic.") && f.endsWith(".rust.print.txt"))) {
        const base = entry.slice(0, -".rust.print.txt".length);
        const go = api.client.apiRequest("printNode", { data: fs.readFileSync(path.join(rustDir, `${base}.rust.bin`)).toString("base64") });
        const rust = fs.readFileSync(path.join(rustDir, entry), "utf8");
        printed++;
        if (go !== rust) {
            failures++;
            console.error(`PRINT MISMATCH ${base}\n  go:   ${JSON.stringify(go)}\n  rust: ${JSON.stringify(rust)}`);
        }
    }
    api.close();
}
console.log(`pinned server printNode of rust synthesized encodings: ${printed} compared`);
console.log(`node client: ${files} encodings, ${lines} lines compared, ${failures} mismatches`);
process.exit(failures === 0 && files > 0 ? 0 : 1);
