// The error prints the object literal's type, `{ a: Q; }`, with the literal as the enclosing declaration. `Q` is not
// imported here, so the node builder looks for a module that re-exports it (getAlternativeContainingModules): first
// among the imports of this file, then among every module of the program, leaf.ts included.
import { getQ } from "./m";

export const x: string = { a: getQ() };
