// `for (const k in obj)` narrows `obj` to non-null inside the loop.
export function keys(obj?: Record<string, number>) {
    for (const k in obj) {
        const n: number = obj[k];
        const s: string = obj[k];
    }
}
