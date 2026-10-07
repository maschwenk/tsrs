// A leaf (notes/mem-free-leaf-files.md): a module that no other file imports and that exports only its own
// declarations. With `--noEmit` the CLI frees its tree once it is checked, before a.ts is checked.
import { getQ } from "./m";

export const fromLeaf = getQ();

export function leaf0(n: number): { value: number; label: string } {
    return { value: n * 2 + 0, label: "leaf0" };
}

export function leaf1(n: number): { value: number; label: string } {
    return { value: n * 2 + 1, label: "leaf1" };
}

export function leaf2(n: number): { value: number; label: string } {
    return { value: n * 2 + 2, label: "leaf2" };
}

export function leaf3(n: number): { value: number; label: string } {
    return { value: n * 2 + 3, label: "leaf3" };
}

export function leaf4(n: number): { value: number; label: string } {
    return { value: n * 2 + 4, label: "leaf4" };
}

export function leaf5(n: number): { value: number; label: string } {
    return { value: n * 2 + 5, label: "leaf5" };
}

export function leaf6(n: number): { value: number; label: string } {
    return { value: n * 2 + 6, label: "leaf6" };
}

export function leaf7(n: number): { value: number; label: string } {
    return { value: n * 2 + 7, label: "leaf7" };
}
