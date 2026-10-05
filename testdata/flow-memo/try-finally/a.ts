declare function work(): void;
export function f(x: string | number | undefined) {
    let y = x;
    try {
        if (y === undefined) return;
        work();
        y = 1;
    } catch {
        y = "caught";
    } finally {
        const t1: never = y;
        if (typeof y === "string") y.length;
    }
    const after: string | number = y;
    const t2: never = y;
    return [y, after];
}
