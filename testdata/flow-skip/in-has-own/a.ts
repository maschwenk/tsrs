// `"k" in o` and `o.hasOwnProperty("k")` narrow `o.k`, a reference longer than the path they name.
interface AnyValue { stringValue?: string | null; boolValue?: boolean }
export function decode(input: AnyValue | null | undefined) {
    if (input == null) return null;
    if ("stringValue" in input) {
        const s: string | null = input.stringValue;
        return s;
    }
    if (input.hasOwnProperty("boolValue")) {
        const b: boolean = input.boolValue;
        return b;
    }
    const v: string = input.stringValue;
    return v;
}
