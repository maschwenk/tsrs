// Expression-statement calls narrow through their effects signature; never-returning calls end paths.
declare function assert(value: unknown): asserts value;
declare function assertIsString(x: unknown): asserts x is string;
declare function fail(): never;
declare function log(...args: unknown[]): void;
class Box {
    value: string | number = 0;
    assertString(): asserts this is { value: string } {}
}

export function value(x: string | undefined) {
    log(x);
    assert(x);
    const s: number = x;
}

export function typed(x: unknown) {
    log(x);
    assertIsString(x);
    const n: number = x;
}

export function viaThis(b: Box) {
    b.assertString();
    const n: number = b.value;
}

export function aliasArgument(x: string | number) {
    const ok = typeof x === "string";
    assert(ok);
    const n: number = x;
}

export function afterInLoop(x: string | number) {
    fail();
    while (true) {
        const s: string = x;
        if (typeof x === "number") break;
    }
}

export function laterCall(x: string | undefined) {
    const before: string = x;
    assert(x);
    const after: number = x;
}
