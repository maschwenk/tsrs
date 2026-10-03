// Codec-lane re-audit probes for parity 385d300 (comparator d5e40b4); uses the checkout's controls/mk.mjs.
import { run } from "./mk.mjs";
const P = { snapshot: 1, project: "p" };
const sym = (id, name, extra = {}) => ({ reference: { kind: 1, id }, name, flags: 2, checkFlags: 0, ...extra });
const q1 = ["getSymbolAtPosition", { ...P, file: "/f.ts", position: 4 }];
const q2 = ["getSymbolsInScope", { ...P, location: "3.80./f.ts", meaning: 2 }];
const tos = id => ["getTypeOfSymbol", { ...P, symbol: { kind: 1, id } }];
const T = (id, v) => ({ id, flags: 128, value: v });
const R = v => ["getTypeAtPosition", { ...P, file: "/f.ts", position: 9 }, { id: 3, flags: 128, value: v }];
const E = ["getTypeAtPosition", { ...P, file: "/f.ts", position: 9 }, "boom", "error"];
const scen = process.argv[2];
// T1: oracle disagreement through kind (run 2 errored) vs through value; candidate matches neither.
if (scen === "t1-run2-error") { run("syn-a", [{ mode: "sync", test: "t", ex: [R("x")] }]); run("syn-a2", [{ mode: "sync", test: "t", ex: [E] }]); run("syn-b", [{ mode: "sync", test: "t", ex: [R("WRONG")] }]); }
if (scen === "t1-run2-value") { run("syn-a", [{ mode: "sync", test: "t", ex: [R("x")] }]); run("syn-a2", [{ mode: "sync", test: "t", ex: [R("y")] }]); run("syn-b", [{ mode: "sync", test: "t", ex: [R("WRONG")] }]); }
if (scen === "t1-run1-error") { run("syn-a", [{ mode: "sync", test: "t", ex: [E] }]); run("syn-a2", [{ mode: "sync", test: "t", ex: [R("y")] }]); run("syn-b", [{ mode: "sync", test: "t", ex: [R("WRONG")] }]); }
if (scen === "t1-run1-error-ok") { run("syn-a", [{ mode: "sync", test: "t", ex: [E] }]); run("syn-a2", [{ mode: "sync", test: "t", ex: [R("y")] }]); run("syn-b", [{ mode: "sync", test: "t", ex: [R("y")] }]); }
// T2: candidate follows run 2's error at exchange 1 (lock run 2), then answers exchange 2 with run 1's value.
if (scen === "t2" || scen === "t2-ctl") {
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q1, sym(1, "x")], [...tos(1), T(7, "tx")]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q1, "boom", "error"], [...tos(1), T(8, "ty")]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q1, "boom", "error"], [...tos(1), T(9, scen === "t2" ? "tx" : "ty")]] }]);
}
// T3: oracle permutes only a nested array; candidate permutes the outer array instead (never observed).
if (scen === "t3" || scen === "t3-ctl") {
    const x = (id, d) => sym(id, "x", { declarations: d }), y = id => sym(id, "y", { declarations: ["40.261./f.ts"] });
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q2, [x(1, ["1.261./f.ts", "2.261./f.ts"]), y(2)]]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q2, [x(1, ["2.261./f.ts", "1.261./f.ts"]), y(2)]]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q2, scen === "t3" ? [y(6), x(5, ["2.261./f.ts", "1.261./f.ts"])] : [x(5, ["2.261./f.ts", "1.261./f.ts"]), y(6)]]] }]);
}
