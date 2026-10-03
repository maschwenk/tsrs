// @ts-check
/**
 * Adds numbers.
 * @param {number} a - first {@link other} value
 * @param {number} [b=1] second
 * @returns {number} the sum
 * @template T
 * @typedef {{ x: number, y?: string }} Point
 * @see {@linkcode Other}
 */
function add(a, b) { return a + b; }
/** @type {(x: string) => void} */
const cb = (x) => {};
/** @enum {string} */
const E = { A: "a" };
module.exports = { add };
