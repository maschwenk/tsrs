// Imported by index.ts: not a leaf, so not freed. Checked after leaf.test.ts.
import { G } from "./lib";

export const x: G<{ a: number }> = { value: { a: 1 } };

// The error prints G<{ a: number; }>.
export const y: G<{ a: string }> = x;
