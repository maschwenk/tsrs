export function f<T extends string | undefined>(x: T, y: T) {
    let z = x;
    if (z) {
        z.length;
        const t1: never = z;
        const s: string = z;
        const t: T = z;
        z = y;
        z.length;
        const u: string = z;
        return [s, t, u];
    }
    return z;
}
