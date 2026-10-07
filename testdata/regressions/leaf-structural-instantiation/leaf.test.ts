// A leaf (notes/mem-free-leaf-files.md): nothing imports it and it exports only its own declarations. With one
// checker it is checked, and with `--noEmit` freed, before other.ts (program order: lib.ts, leaf.test.ts, other.ts,
// index.ts). It instantiates G with an object literal type of its own; other.ts then instantiates G with a
// structurally identical one. The checker's caches are keyed by type ids, not structure, so other.ts's error must not
// reach this file's (freed) type.
import { G } from "./lib";

export const fromLeaf: G<{ a: number }> = { value: { a: 1 } };

export function readLeaf(g: G<{ a: number }>): number {
    return g.value.a;
}
