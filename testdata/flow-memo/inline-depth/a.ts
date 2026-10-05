export function f(x: string | number | boolean | null | undefined | bigint | symbol) {
    const a1 = typeof x === "string";
    const a2 = a1 || typeof x === "number";
    const a3 = a2 || typeof x === "boolean";
    const a4 = a3 || x === null;
    const a5 = a4 || x === undefined;
    const a6 = a5 || typeof x === "bigint";
    const a7 = a6;
    if (a7) {
        const t1: never = x;
    } else {
        const t2: never = x;
    }
    if (a5) {
        const s: string | number | boolean | null | undefined = x;
        const t3: never = x;
        return s;
    }
    return x;
}
