// An assignment to the reference gives the declared type unless that is a union or a literal type.
declare function pick(): string | number;
export function unionAssigned() {
    let x: string | number = pick();
    x = 1;
    const s: string = x;
}

export function literalCompound() {
    let n = 1 as 1 | 2;
    n += 1;
    const s: string = n;
}

export function plainAssigned() {
    let s: string = "";
    s = "b";
    const n: number = s;
}

export function destructured(o: { a: string | number }) {
    let a: string | number;
    ({ a } = { a: 1 });
    const s: string = a;
}

export function parameterAssigned(x: string | number) {
    x = 1;
    const s: string = x;
}

export function propertyAssigned(o: { v: string | number }) {
    o.v = 1;
    const s: string = o.v;
}
