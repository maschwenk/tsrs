import type { MappedOptions, Options, WithValueOf } from "./options"

export const ok: Options = { p3: "a", p60: "b" }

// Two failing properties, listed in the opposite order to the target's: the first in target order is reported.
export const twoFailures: Options = { p60: 1, p3: 2 }

// Not fresh: no excess property check, the same first failure.
const loose = { p40: 5, p2: true, extra: "x" }
export const notFresh: Options = loose

// A class instance, with two failing properties in the opposite order to the target's.
class Settings {
	p5 = "x"
	p7 = 7
	p1 = 1
	private hidden = 1
}
export const fromClass: Options = new Settings()

// No common property: the weak type check.
export const weak: Options = { zzz: 1 }

// `toString` comes from `Object` for any object literal, and is not a number.
export const viaObject: MappedOptions = { p1: "a" }

// Ok when the literal declares it.
export const declared: MappedOptions = { p1: "a", toString: 1 }

// `valueOf` comes from `Object` and is not a string; the target is large enough for the source-keyed walk.
export const valueOfFromObject: WithValueOf = { q1: "a" }
