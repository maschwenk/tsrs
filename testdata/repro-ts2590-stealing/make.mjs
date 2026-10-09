// Usage: node make.mjs <dir> <slow> [--no-filler]
// Writes a project where x/a.ts and x/zz.ts reach the same too-complex type. a.ts ignores the TS2590, so zz.ts reports
// it only when a different checker runs it. <slow> sets how long a.ts takes to check; tune it so the checker that owns
// a.ts and zz.ts finishes its queue at about the same time as the other checker (see README).
import * as fs from "node:fs"
import * as path from "node:path"

const [dir, slowArg, fillerArg] = process.argv.slice(2)
const slow = Number(slowArg)
// --no-filler: only map.ts, x/a.ts and x/zz.ts, checked in that order, for comparing checker counts in tsgo and tsrs.
const filler = fillerArg !== "--no-filler"
const write = (file, text) => {
	fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true })
	fs.writeFileSync(path.join(dir, file), text)
}

// Three keys, each mapped to a union of 50 object types: the write constraint of ModelMap[T] is their intersection,
// 50^3 = 125,000 constituents, over the 100,000 limit.
let map = ""
for (const key of ["k0", "k1", "k2"]) {
	map += `export type ${key.toUpperCase()} = ${Array.from({ length: 50 }, (_, i) => `{ ${key}_${i}: ${i} }`).join(" | ")}\n`
}
map += "export type ModelMap = { k0: K0; k1: K1; k2: K2 }\n"
map += "export type Key = keyof ModelMap\n"
map += "export interface GetModel {\n\t<T extends Key>(key: T): ModelMap[T] | undefined\n}\n"
write("map.ts", map)

const header = 'import type { GetModel, Key, ModelMap } from "../map"\ndeclare function lookup(key: Key): ModelMap[Key] | undefined\n'
write(
	"x/a.ts",
	`${header}// @ts-ignore\nexport const getA: GetModel = key => lookup(key)\n` +
		'type Build<N extends number, A extends unknown[] = []> = A["length"] extends N ? A : Build<N, [...A, A["length"]]>\n' +
		`export type Slow = Build<${slow}>\n`,
)
// Imports a.ts so the locality placement queues it with a.ts; named to sort last, so it ends that queue.
write("x/zz.ts", `${header}import { getA } from "./a"\nexport const again = getA\nexport const getB: GetModel = key => lookup(key)\n`)

// Filler: x/ files import a.ts (same queue as a.ts and zz.ts), w/ files don't.
const body = Array.from({ length: 6 }, (_, j) => `\tconst v${j} = [{ a: x + ${j}, b: String(x) }].map(o => ({ ...o, a: o.a * 2 }))\n`).join("")
for (let i = 0; filler && i < 800; i++) {
	const n = String(i).padStart(4, "0")
	const fn = `export function f${i}(x: number): number {\n${body}\treturn x\n}\n`
	write(`w/fill${n}.ts`, fn)
	write(`x/fill${n}.ts`, `import { getA } from "./a"\nexport const ref = getA\n${fn}`)
}
write(
	"tsconfig.json",
	JSON.stringify({ compilerOptions: { strict: true, noEmit: true, target: "es2022", module: "esnext", moduleResolution: "bundler" }, include: ["**/*.ts"] }, undefined, "\t") + "\n",
)
