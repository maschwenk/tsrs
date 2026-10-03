#!/usr/bin/env node
// Regression controls for compare-responses.mjs. CE1-CE5 come from the codec lane's independent audit of parity
// 4c50d93 (scen.mjs/mk.mjs copied verbatim except mk.mjs's output path; originals and their run.out are kept in
// the audit artifact). scen-extra.mjs adds parity-lane controls. Every case states the required method status;
// the script exits 1 if any case does not hold.
//
//   node tools/node-api/controls/run-controls.mjs [--comparator <path>]

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const comparator = path.resolve(argv[argv.indexOf("--comparator") + 1] && argv.includes("--comparator") ? argv[argv.indexOf("--comparator") + 1] : path.join(here, "..", "compare-responses.mjs"));
const out = path.join(here, "..", ".work", "controls");
const VERIFIED = ["equal"];
const NOT_VERIFIED = ["differs", "inconclusive"];

// [label, scenario script, scenario, comparator args, method, accepted statuses]
const cases = [
    ["CE1b order fallback must not sort nested declarations", "scen.mjs", "ce1b", ["--a2", "syn-a2"], "getSymbolsInScope", ["differs"]],
    ["CE1b control: same elements, oracle order", "scen.mjs", "ce1b-ctl", ["--a2", "syn-a2"], "getSymbolsInScope", VERIFIED],
    ["CE2 (ce5) identity break inside an unordered result", "scen.mjs", "ce5", ["--a2", "syn-a2"], "getSymbolsInScope", ["differs"]],
    ["CE2 (ce5) control: consistent identity", "scen.mjs", "ce5-ctl", ["--a2", "syn-a2"], "getSymbolsInScope", VERIFIED],
    ["CE3 request difference on the unordered path", ["scen.mjs", "ce1"], "ce3", ["--a2", "syn-a2"], "getSignaturesOfType", ["differs"]],
    // Known limit, not a false pass of the order relaxation: the swapped parameters are fresh handles that occur
    // nowhere else, so swapping them is indistinguishable from renaming them (strictly equal under the bijection).
    ["CE1 swapped never-reused parameter handles (unobservable)", "scen.mjs", "ce1", ["--a2", "syn-a2"], "getSignaturesOfType", VERIFIED],
    ["CE1 control", ["scen.mjs", "ce1"], "ce1-ctl", ["--a2", "syn-a2"], "getSignaturesOfType", VERIFIED],
    ["CE4 (ce2) candidate matches neither oracle run", "scen.mjs", "ce2", ["--a2", "syn-a2"], "getTypeAtPosition", NOT_VERIFIED],
    ["CE5 (ce4) non-canonical base64 strings are distinct", "scen.mjs", "ce4", [], "getSourceFile", ["differs"]],
    ["CE5 (ce4) control", "scen.mjs", "ce4-ctl", [], "getSourceFile", ["differs"]],
    ["user property __k@1 is not counter-renamed", "scen-extra.mjs", "ce6", [], "getPropertiesOfType", ["differs"]],
    ["user property __k@1 control", "scen-extra.mjs", "ce6-ctl", [], "getPropertiesOfType", VERIFIED],
    ["unique symbol counter suffix is a renaming", "scen-extra.mjs", "ce11-ctl", [], "getPropertiesOfType", VERIFIED],
    ["missing candidate process fails closed", "scen-extra.mjs", "ce7", [], "getTypeAtPosition", ["inconclusive"]],
    ["differing >4 MiB hash is inconclusive, not equal", "scen-extra.mjs", "ce8", [], "getSourceFile", ["inconclusive"]],
    ["sync binary AST identical", "scen-extra.mjs", "ce9", [], "getSourceFile", VERIFIED],
    ["sync binary AST drift control", "scen-extra.mjs", "ce9", ["--drift", "getSourceFile"], "getSourceFile", ["differs"]],
    ["legit reorder where oracle runs disagree on order", "scen-extra.mjs", "ce10-ctl", ["--a2", "syn-a2"], "getSymbolsInScope", VERIFIED],
    // Codec re-audit of c3f1717 (scen2.mjs, verbatim except the mk.mjs import and its work-tree constant).
    ["N1 run2-then-use: wrong answer after a run-2 acceptance", "scen2.mjs", "run2-then-use", ["--a2", "syn-a2"], "getTypeOfSymbol", NOT_VERIFIED],
    ["N1 run2-then-use: the run-2 exchange itself", "scen2.mjs", "run2-then-use", ["--a2", "syn-a2"], "getSymbolsInScope", VERIFIED],
    ["N1 control: correct answer on the run-2 history", "scen2.mjs", "run2-then-use-ctl", ["--a2", "syn-a2"], "getTypeOfSymbol", VERIFIED],
    ["N3 extra trailing candidate exchange", "scen2.mjs", "extra-tail2", [], "getTypeOfSymbol", NOT_VERIFIED],
    ["N3 extra candidate process", "scen2.mjs", "extra-proc", [], "getSymbolAtPosition", NOT_VERIFIED],
    ["N3 control", "scen2.mjs", "extra-proc-ctl", [], "getSymbolAtPosition", VERIFIED],
    ["N2 chained handles, valid renaming in a third order", "scen2.mjs", "greedy", ["--a2", "syn-a2"], "getSymbolsInScope", VERIFIED],
    ["reorder keeps identity for later use", "scen2.mjs", "unordered-then-use", ["--a2", "syn-a2"], "getTypeOfSymbol", VERIFIED],
    ["reorder then swapped handle", "scen2.mjs", "unordered-then-use-swap", ["--a2", "syn-a2"], "getTypeOfSymbol", NOT_VERIFIED],
    ["budget: large correct reorder", "scen2.mjs", "bucket", ["--a2", "syn-a2"], "getSymbolsInScope", ["inconclusive"]],
    ["budget: large wrong reorder", "scen2.mjs", "bucket-wrong", ["--a2", "syn-a2"], "getSymbolsInScope", ["inconclusive"]],
    ["budget: small correct reorder", "scen2.mjs", "bucket-small", ["--a2", "syn-a2"], "getSymbolsInScope", VERIFIED],
    ["budget: small wrong reorder", "scen2.mjs", "bucket-small-wrong", ["--a2", "syn-a2"], "getSymbolsInScope", ["differs"]],
    ["N4 async-only binary method from the schema", "scen2.mjs", "async-binary-only", [], "getSourceFile", VERIFIED],
];

let failed = 0;
const lines = [];
for (const [label, script, scen, args, method, accepted] of cases) {
    // A scenario may need another scenario's oracle files first (CE3/CE1-ctl reuse ce1's syn-a/syn-a2).
    if (Array.isArray(script)) execFileSync(process.execPath, [path.join(here, script[0]), script[1]]);
    const file = Array.isArray(script) ? script[0] : script;
    for (const l of ["syn-a2"]) if (!args.includes(l)) fs.rmSync(path.join(here, "..", ".work", l), { recursive: true, force: true });
    execFileSync(process.execPath, [path.join(here, file), scen]);
    const dir = path.join(out, `${scen}${args.join("_").replace(/[^\w]/g, "")}`);
    try {
        execFileSync(process.execPath, [comparator, "--a", "syn-a", "--b", "syn-b", ...args, "--out", dir], { stdio: ["ignore", "ignore", "pipe"] });
    }
    catch (e) {
        failed++;
        lines.push(`FAIL ${label}: comparator error: ${String(e.stderr ?? e.message).trim().split("\n")[0]}`);
        continue;
    }
    const report = JSON.parse(fs.readFileSync(path.join(dir, "compare.json"), "utf8"));
    const row = report.rows.find(r => r.method === method);
    const ok = accepted.includes(row.status);
    if (!ok) failed++;
    const detail = { okEqual: row.okEqual ?? 0, strictEqual: row.strictEqual ?? 0, differ: row.differ ?? 0, equalUnordered: row.equalUnordered ?? 0, oracleUnstable: row.oracleUnstable ?? 0, unverified: row.unverified ?? 0 };
    lines.push(`${ok ? "ok  " : "FAIL"} ${label}: ${method} ${row.status} (want ${accepted.join("|")}) ${JSON.stringify(detail)}`);
}
console.log(lines.join("\n"));
console.log(failed ? `${failed} control(s) failed` : `all ${cases.length} controls hold`);
process.exit(failed ? 1 : 0);
