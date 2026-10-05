export function f(x: string | number) {
    let a = x;
    let b = a;
    while (typeof a === "string") {
        b = a;
        a = b.length > 3 ? b : b.length;
    }
    const c: number = a;
    const t1: never = b;
    return [b, c];
}
export function g() {
    let v = h();
    function h() { return v ? 1 : 2; }
    if (v) {
        const t2: never = v;
        return v;
    }
    return v;
}
