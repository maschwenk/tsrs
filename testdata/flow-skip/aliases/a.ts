// Conditions that narrow a reference without naming it: the binder's index must close over these aliases.
type Shape = { kind: "a"; a: number } | { kind: "b"; b: string };
declare function isString(x: unknown): x is string;
declare const shape: Shape;
declare const pair: [kind: "x", n: number] | [kind: "y", s: string];

export function sameScope(x: string | number) {
    const ok = typeof x === "string";
    if (ok) { const s: number = x; }
    const notOk = !ok;
    if (notOk) { const n: string = x; }
}

export function moduleLevelAfterUse() {
    if (kind === "a") { const n: string = shape.a; }
    if (isA) { const n: string = shape.a; }
}
const { kind } = shape;
const isA = shape.kind === "a";

export function accessAlias(s: Shape) {
    const k = s.kind;
    if (k === "b") { const n: number = s.b; }
}

export function arrayAlias() {
    const [tag] = pair;
    if (tag === "y") { const n: number = pair[1]; }
}

export function transitive(x: string | number) {
    const a = typeof x === "string";
    const b = a;
    if (b) { const n: number = x; }
}

export function predicateAlias(x: unknown) {
    const ok = isString(x);
    if (ok) { const n: number = x; }
}

export function outerScope(x: string | number) {
    const ok = typeof x === "number";
    return () => {
        if (ok) { const s: string = x; }
    };
}

export function shadowed(x: string | number) {
    {
        const ok = typeof x === "string";
        if (ok) { const n: number = x; }
    }
    const ok = 1;
    if (ok) { const n: number = x; }
}
