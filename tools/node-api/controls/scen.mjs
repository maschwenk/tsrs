import { run } from "./mk.mjs";
const P = { snapshot: 1, project: "p" };
const typeEx = id => ["getTypeAtPosition", { ...P, file: "/f.ts", position: 3 }, { id, flags: 1, value: null }];
const sigs = (ids, order, swapParams) => {
    const s1 = { id: ids[0], flags: 0, declaration: "5.170./f.ts", parameters: swapParams ? [{ id: ids[3] }, { id: ids[2] }] : [{ id: ids[2] }, { id: ids[3] }] };
    const s2 = { id: ids[1], flags: 0, declaration: "9.170./f.ts", parameters: [{ id: ids[4] }] };
    return order ? [s1, s2] : [s2, s1];
};
const S = (tid, kind, res) => ["getSignaturesOfType", { ...P, type: tid, kind }, res];
const scen = process.argv[2];
if (scen === "ce1") {
    // oracle order nondeterministic (A vs A2 outer order); B keeps A's outer order but swaps parameter order
    run("syn-a", [{ mode: "sync", test: "t", ex: [typeEx(7), S(7, 0, sigs([1, 2, 10, 11, 12], true, false))] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [typeEx(7), S(7, 0, sigs([1, 2, 10, 11, 12], false, false))] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [typeEx(70), S(70, 0, sigs([101, 102, 110, 111, 112], true, true))] }]);
}
if (scen === "ce1-ctl") {
    // same oracles, B identical to A (up to ids): must be equal
    run("syn-b", [{ mode: "sync", test: "t", ex: [typeEx(70), S(70, 0, sigs([101, 102, 110, 111, 112], true, false))] }]);
}
if (scen === "ce3") {
    // B sends a different request (kind 1 instead of 0) and answers like A: request difference ignored
    run("syn-b", [{ mode: "sync", test: "t", ex: [typeEx(70), S(70, 1, sigs([101, 102, 110, 111, 112], true, false))] }]);
}
if (scen === "ce2") {
    // oracle runs disagree in a value (not order); B matches neither, plus one fully equal exchange -> status?
    const R = v => ["getTypeAtPosition", { ...P, file: "/f.ts", position: 9 }, { id: 3, flags: 128, value: v }];
    run("syn-a", [{ mode: "sync", test: "t", ex: [typeEx(7), R("x")] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [typeEx(7), R("y")] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [typeEx(70), R("WRONG")] }]);
}
if (scen === "ce4" || scen === "ce4-ctl") {
    const data = v => ["getSourceFile", { ...P, file: "/f.ts" }, { data: v }, "response", 1];
    run("syn-a", [{ mode: "async", test: "t", ex: [data("AAAAAAAAAAAAAAE=")] }]);
    run("syn-b", [{ mode: "async", test: "t", ex: [data(scen === "ce4" ? "AAAAAAAAAAAAAAF=" : "AAAAAAAAAAAAAAI=")] }]);
}
if (scen === "ce1b" || scen === "ce1b-ctl") {
    // getSymbolsInScope: oracle runs disagree on element order; B keeps A's order but reverses one symbol's
    // `declarations` (semantic source order) -- only visible if declarations order is compared.
    const sym = (id, name, decls) => ({ reference: { kind: 1, id }, name, flags: 2, checkFlags: 0, declarations: decls });
    const q = ["getSymbolsInScope", { ...P, location: "3.80./f.ts", meaning: 2 }];
    const x = (id, rev) => sym(id, "x", rev ? ["30.261./f.ts", "12.261./f.ts"] : ["12.261./f.ts", "30.261./f.ts"]);
    const y = id => sym(id, "y", ["40.261./f.ts"]);
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q, [x(1), y(2)]]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q, [y(2), x(1)]]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q, [x(5), y(6)].map((s, i) => i === 0 && scen === "ce1b" ? x(5, true) : s)]] }]);
}
if (scen === "ce5" || scen === "ce5-ctl") {
    // identity: exchange 1 introduces symbol x (A id 1, B id 5); exchange 2 is order-nondeterministic in the oracle,
    // and B reports x with a DIFFERENT id (6) there -- a handle identity break.
    const sym = (id, name) => ({ reference: { kind: 1, id }, name, flags: 2, checkFlags: 0 });
    const q1 = ["getSymbolAtPosition", { ...P, file: "/f.ts", position: 4 }];
    const q2 = ["getSymbolsInScope", { ...P, location: "3.80./f.ts", meaning: 2 }];
    run("syn-a", [{ mode: "sync", test: "t", ex: [[...q1, sym(1, "x")], [...q2, [sym(1, "x"), sym(2, "y")]]] }]);
    run("syn-a2", [{ mode: "sync", test: "t", ex: [[...q1, sym(1, "x")], [...q2, [sym(2, "y"), sym(1, "x")]]] }]);
    run("syn-b", [{ mode: "sync", test: "t", ex: [[...q1, sym(5, "x")], [...q2, [sym(scen === "ce5" ? 6 : 5, "x"), sym(7, "y")]]] }]);
}
