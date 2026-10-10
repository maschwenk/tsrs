// A walk continues from a function expression's start into the enclosing function's flow.
export function captured(x: string | number) {
    if (typeof x === "string") {
        const f = () => { const n: number = x; };
        const g = function () { const n: number = x; };
        const o = { m() { const n: number = x; } };
        return [f, g, o];
    }
    return [];
}

export class C {
    kind: "a" | "b" = "a";
    m() {
        if (this.kind === "a") {
            const f = () => { const n: number = this.kind; };
            return f;
        }
        return undefined;
    }
}

export function nested(x: string | number) {
    if (typeof x !== "number") return;
    return () => () => { const s: string = x; };
}
