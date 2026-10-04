// Codec-lane re-audit scenarios for parity 0333222 (comparator c8ffcd0); uses the checkout's controls/mk.mjs.
import { run } from "./mk.mjs";
const P = { snapshot: 1, project: "p" };
const sym = (id, name) => ({ reference: { kind: 1, id }, name, flags: 2, checkFlags: 0 });
const q1 = ["getSymbolAtPosition", { ...P, file: "/f.ts", position: 4 }];
const tos = id => ["getTypeOfSymbol", { ...P, symbol: { kind: 1, id } }];
const T = (id, v) => ({ id, flags: 128, value: v });
const scen = process.argv[2];
if (scen === "n1b" || scen === "n1b-ctl") {
    // Exchange 1: oracle run 2 errored (not aligned), candidate matches run 1 -> accepted on run 1 without a lock.
    // Exchange 2: candidate follows run 1's request but answers with run 2's value ("ty"); correct is "tx".
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q1, sym(1, "x")], [...tos(1), T(7, "tx")]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q1, "boom", "error"], [...tos(2), T(8, "ty")]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q1, sym(5, "x")], [...tos(5), T(9, scen === "n1b" ? "ty" : "tx")]] }]);
}
if (scen === "n4-wrong" || scen === "n4-ok") {
    // Async-only binary method (schema-pinned): same path placeholder, different AST bytes must differ.
    const tree = new URL("../.work", import.meta.url).pathname.replace(/\/$/, "");
    const enc = (label, body) => Buffer.from(`AST:${tree}/${label}/tree/src/a.ts:${body}`).toString("base64");
    const d = (label, body) => ["getSourceFile", { ...P, file: "/f.ts" }, { data: enc(label, body) }, "response", 1];
    run("syn-a", [{ mode: "async", test: "t", ex: [d("syn-a", "END")] }]);
    run("syn-b", [{ mode: "async", test: "t", ex: [d("syn-b", scen === "n4-ok" ? "END" : "EnD")] }]);
}
if (scen === "same-key-order" || scen === "same-key-order-wrong") {
    // Three structurally identical elements, each tied to a parent handle the client received earlier (so they are
    // distinguishable under the process mapping). Oracle runs: [e0,e1,e2] and [e2,e1,e0]; candidate: [e1,e2,e0]
    // renamed consistently (a third order of the same elements). Wrong variant: two elements claim one parent.
    const intro = base => ["getParents", { ...P }, [0, 1, 2].map(i => ({ id: base + i }))];
    const el = (r, p) => ({ reference: { kind: 1, id: r }, name: "k", flags: 2, checkFlags: 0, parent: { id: p } });
    const q2 = ["getSymbolsInScope", { ...P, location: "3.80./f.ts", meaning: 2 }];
    const A = [el(1, 1000), el(2, 1001), el(3, 1002)];
    run("syn-a", [{ mode: "sync", test: "t", ex: [intro(1000), [...q2, A]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [intro(1000), [...q2, [...A].reverse()]] }]);
    const B = scen === "same-key-order" ? [el(12, 5001), el(13, 5002), el(11, 5000)] : [el(12, 5001), el(13, 5001), el(11, 5000)];
    run("syn-b", [{ mode: "sync", test: "t", ex: [intro(5000), [...q2, B]] }]);
}
if (scen === "chain-use" || scen === "chain-use-swap") {
    // N2 chained renaming in a third order (backtracking needed), then the client asks for e1 by its handle.
    // The first-fit branch (abandoned) bound u<->12; the accepted branch binds u<->11. A later request must use 11.
    const el = (r, p) => ({ reference: { kind: 1, id: r }, name: "e", flags: 2, checkFlags: 0, parent: { id: p } });
    const x = (r, p) => ({ reference: { kind: 1, id: r }, name: "x", flags: 2, checkFlags: 0, parent: { id: p } });
    const q2 = ["getSymbolsInScope", { ...P, location: "3.80./f.ts", meaning: 2 }];
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q1, sym(99, "p")], [...q2, [x(50, 99), el(1, 2), el(2, 3)]], [...tos(1), T(7, "t1")]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q1, sym(99, "p")], [...q2, [el(2, 3), el(1, 2), x(50, 99)]], [...tos(1), T(7, "t1")]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q1, sym(199, "p")], [...q2, [x(60, 199), el(12, 13), el(11, 12)]], [...tos(scen === "chain-use" ? 11 : 12), T(9, "t1")]] }]);
}
