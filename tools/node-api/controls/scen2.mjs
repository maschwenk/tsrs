// Codec-lane re-audit scenarios for compare-responses.mjs at parity c3f1717 (uses the parity checkout's mk.mjs).
import { run } from "./mk.mjs";
const P = { snapshot: 1, project: "p" };
const sym = (id, name, extra = {}) => ({ reference: { kind: 1, id }, name, flags: 2, checkFlags: 0, ...extra });
const q1 = ["getSymbolAtPosition", { ...P, file: "/f.ts", position: 4 }];
const q2 = ["getSymbolsInScope", { ...P, location: "3.80./f.ts", meaning: 2 }];
const tos = id => ["getTypeOfSymbol", { ...P, symbol: { kind: 1, id } }];
const scen = process.argv[2];
const T = (id, v) => ({ id, flags: 128, value: v });
if (scen === "extra-tail" || scen === "extra-tail-ctl") {
    // Candidate process sends one extra request after the oracle's last exchange.
    const ex = [[...q1, sym(1, "x")]];
    run("syn-a", [{ mode: "sync", test: "t", ex }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: scen === "extra-tail" ? [[...q1, sym(5, "x")], [...tos(5), T(9, "WRONG")]] : [[...q1, sym(5, "x")]] }]);
}
if (scen === "extra-proc" || scen === "extra-proc-ctl") {
    // Candidate spawns an extra server process in the same test (e.g. a client reconnect) with its own traffic.
    const ex = [[...q1, sym(1, "x")]];
    run("syn-a", [{ mode: "sync", test: "t", ex }]);
    run("syn-b", scen === "extra-proc" ? [{ mode: "sync", test: "t", ex: [[...q1, sym(5, "x")]] }, { mode: "sync", test: "t", ex: [[...q1, sym(9, "WRONG")]] }] : [{ mode: "sync", test: "t", ex: [[...q1, sym(5, "x")]] }]);
}
if (scen === "reorder-then-use" || scen === "reorder-then-use-swap") {
    // Exchange 2 is order-relaxed (oracle runs disagree on order); exchange 3 uses a handle bound by the reorder.
    // Elements are distinguishable (x has a declaration, y another), so a swapped handle in exchange 3 must differ.
    const x = (id) => sym(id, "x", { declarations: ["12.261./f.ts"] });
    const y = (id) => sym(id, "y", { declarations: ["40.261./f.ts"] });
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q2, [x(1), y(2)]], [...tos(1), T(7, "tx")]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q2, [y(2), x(1)]], [...tos(1), T(7, "tx")]] }]);
    // B's third order equals neither oracle order exactly? only 2 elements: use B order == A2 order but different ids
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q2, [y(6), x(5)]], [...tos(scen === "reorder-then-use" ? 5 : 6), T(9, "tx")]] }]);
}
if (scen.startsWith("budget")) {
    // 260 structurally identical elements (handles only) in an order-relaxed array: about n^2/2 > 20000 attempts.
    const n = scen.includes("small") ? 150 : 260;
    const el = (id, i) => ({ reference: { kind: 1, id }, name: "k", flags: 2, checkFlags: 0, parent: { id: 1000 + i } });
    const intro = ["getSymbolsOfParents", { ...P }, Array.from({ length: n }, (_, i) => ({ id: 1000 + i }))];
    const introB = ["getSymbolsOfParents", { ...P }, Array.from({ length: n }, (_, i) => ({ id: 5000 + i }))];
    const A = Array.from({ length: n }, (_, i) => el(i + 1, i));
    const A2 = [...A].reverse();
    const elB = (id, i) => ({ reference: { kind: 1, id }, name: "k", flags: 2, checkFlags: 0, parent: { id: 5000 + i } });
    const Bf = A.map((e, i) => elB(3000 + i, i));
    const B = [...Bf.slice(1), Bf[0]]; // a third order (rotation): needs the order relaxation
    if (scen.includes("wrong")) B[0] = elB(3001, 7); // parent handle duplicated -> not a valid permutation
    run("syn-a", [{ mode: "sync", test: "t", ex: [intro, [...q2, A]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [intro, [...q2, A2]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [introB, [...q2, B]] }]);
}
if (scen === "async-binary-only") {
    // getSourceFile appears only on the async channel ({"data": base64}); no sync capture makes it a binary method.
    const tree = new URL("../.work", import.meta.url).pathname.replace(/\/$/, "");
    const enc = (label) => Buffer.from(`AST:${tree}/${label}/tree/src/a.ts:END`).toString("base64");
    const d = label => ["getSourceFile", { ...P, file: "/f.ts" }, { data: enc(label) }, "response", 1];
    run("syn-a", [{ mode: "async", test: "t", ex: [d("syn-a")] }]);
    run("syn-b", [{ mode: "async", test: "t", ex: [d("syn-b")] }]);
}
if (scen === "extra-tail2") {
    // getTypeOfSymbol is compared (equal) once; the candidate then sends an extra trailing getTypeOfSymbol with a wrong answer.
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q1, sym(1, "x")], [...tos(1), T(7, "tx")]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q1, sym(5, "x")], [...tos(5), T(9, "tx")], [...tos(5), T(9, "WRONG")]] }]);
}
if (scen === "run2-then-use" || scen === "run2-then-use-ctl") {
    // Exchange 1 matches oracle run 2 exactly (order [y, x]); the primary bijection learns nothing from it.
    // Exchange 2: every client asks for element [0]. A asks x, A2 and B ask y. B answers with x's type (wrong).
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q2, [sym(1, "x"), sym(2, "y")]], [...tos(1), T(7, "tx")]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q2, [sym(2, "y"), sym(1, "x")]], [...tos(2), T(8, "ty")]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q2, [sym(6, "y"), sym(5, "x")]], [...tos(6), T(9, scen === "run2-then-use" ? "tx" : "ty")]] }]);
}
if (scen === "unordered-then-use" || scen === "unordered-then-use-swap") {
    // Exchange 1 goes through the order relaxation (B uses a third order); exchange 2 asks x by handle.
    const x = id => sym(id, "x"), y = id => sym(id, "y"), z = id => sym(id, "z");
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q2, [x(1), y(2), z(3)]], [...tos(1), T(7, "tx")]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q2, [z(3), x(1), y(2)]], [...tos(3), T(8, "tz")]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q2, [y(6), z(7), x(5)]], [...tos(scen === "unordered-then-use" ? 5 : 6), T(9, "tx")]] }]);
}
if (scen.startsWith("bucket")) {
    // Two structural buckets ("a"/"b") whose order the oracle runs swap; elements are tied to parents introduced
    // earlier, and the candidate lists each bucket in reverse, so greedy matching needs ~ (n/2)^2/2 attempts per bucket.
    const n = scen.includes("small") ? 100 : 600;
    const intro = (base) => ["getParents", { ...P }, Array.from({ length: n }, (_, i) => ({ id: base + i }))];
    const el = (id, i, pbase) => ({ reference: { kind: 1, id }, name: i < n / 2 ? "a" : "b", flags: 2, checkFlags: 0, parent: { id: pbase + i } });
    const A = Array.from({ length: n }, (_, i) => el(i + 1, i, 1000));
    const A2 = [...A.slice(n / 2), ...A.slice(0, n / 2)];
    const Bel = Array.from({ length: n }, (_, i) => el(3000 + i, i, 5000));
    const B = [...Bel.slice(n / 2).reverse(), ...Bel.slice(0, n / 2).reverse()];
    if (scen.includes("wrong")) B[0] = { ...B[0], parent: { id: 5000 + n - 2 } }; // two b elements now claim one parent
    run("syn-a", [{ mode: "sync", test: "t", ex: [intro(1000), [...q2, A]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [intro(1000), [...q2, A2]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [intro(5000), [...q2, B]] }]);
}
if (scen === "greedy") {
    // Order relaxation with chained once-only handles: e1 = {ref u, parent v}, e2 = {ref v, parent w}.
    // The candidate is a renaming of the oracle in a third order; a valid matching exists (e1<->f1, e2<->f2),
    // but first-fit tries f2 for e1 first, commits u<->12, v<->13, and then e2 has no partner.
    const el = (r, p) => ({ reference: { kind: 1, id: r }, name: "e", flags: 2, checkFlags: 0, parent: { id: p } });
    const x = r => ({ reference: { kind: 1, id: r }, name: "x", flags: 2, checkFlags: 0, parent: { id: 99 } });
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q1, sym(99, "p")], [...q2, [x(50), el(1, 2), el(2, 3)]]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q1, sym(99, "p")], [...q2, [el(2, 3), el(1, 2), x(50)]]] }]);
    const xB = r => ({ ...x(r), parent: { id: 199 } });
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q1, sym(199, "p")], [...q2, [xB(60), el(12, 13), el(11, 12)]]] }]);
}
