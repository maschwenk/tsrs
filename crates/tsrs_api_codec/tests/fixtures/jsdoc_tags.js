/**
 * @import { A, B } from "./types"
 * @typedef {Object} Shape
 * @property {number} x
 * @property {string} [label]
 * @callback Handler
 * @param {Event} ev
 * @returns {void}
 */
/**
 * @template T
 * @param {?number} a nullable
 * @param {!string} b non-null
 * @param {number=} c optional
 * @param {...number} d variadic
 * @param {*} e all
 * @param {{ p: number }} f literal
 * @throws {Error} sometimes
 * @deprecated use other
 * @see {@linkplain Other the other}
 * @this {Window}
 */
function f(a, b, c, ...d) {}
/**
 * @augments Base
 * @implements {Iface}
 */
class C extends Base {
    /** @public */ a = 1;
    /** @private */ b = 2;
    /** @protected */ c = 3;
    /** @readonly */ d = 4;
    /** @override */ e() {}
    /**
     * @overload
     * @param {string} x
     * @returns {string}
     */
    /**
     * @param {any} x
     */
    g(x) { return x; }
}
/** @satisfies {Shape} */
const s = { x: 1 };
/** @type {import("./types").A} */
let t;
