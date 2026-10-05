export function f(c: boolean, d: boolean) {
    let x;
    if (c) {
        x = { message: "a", n: 1 };
    } else {
        x = { message: "b", n: 2 };
    }
    if (d) {
        const t1: never = x;
    }
    const y = x;
    const t2: never = x;
    let z = [];
    z.push(1);
    if (c) z.push("s");
    const t3: never = z;
    const w = z;
    const t4: never = [x, y, w];
    return [x, y, z, w];
}
