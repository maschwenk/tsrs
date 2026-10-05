interface MW<D> { serialize(d: D, ctx: object): D | Promise<D>; }
export class Serializer<D> {
    mws: MW<D>[] = [];
    serialize(obj: D | Promise<D>, context: object): Promise<D> {
        let current = obj;
        for (const middleware of this.mws) {
            if (current && typeof (current as Promise<D>).then === "function") {
                current = (current as Promise<D>).then((data) => data && middleware.serialize(data, context));
            } else if (current) {
                try {
                    current = middleware.serialize(current, context);
                } catch (err) {
                    current = Promise.reject(err);
                }
            } else {
                break;
            }
        }
        return current as Promise<D>;
    }
}
export function nested(xs: (string | number | undefined)[][]) {
    let last: string | number | undefined;
    outer: for (const row of xs) {
        for (const x of row) {
            if (x === undefined) continue outer;
            if (typeof last === "string" && typeof x === "number") break outer;
            last = x;
        }
        const t1: never = last;
    }
    const n: string | number | undefined = last;
    const t2: never = last;
    return [last, n];
}
