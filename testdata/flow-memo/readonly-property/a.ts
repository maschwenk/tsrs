interface A { readonly kind: "a"; readonly v: string | undefined }
interface B { kind: "b"; v: string | undefined }
export function f(o: A | B) {
    const isA = o.kind === "a";
    const hasV = o.v !== undefined;
    if (isA) {
        if (hasV) {
            const t1: never = o.v;
            const s: string = o.v;
            return s;
        }
    } else if (hasV) {
        const t2: never = o.v;
    }
    return o.v;
}
