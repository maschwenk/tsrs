import type { GetModel, Key, ModelMap } from "../map"
declare function lookup(key: Key): ModelMap[Key] | undefined
// @ts-ignore
export const getA: GetModel = key => lookup(key)
type Build<N extends number, A extends unknown[] = []> = A["length"] extends N ? A : Build<N, [...A, A["length"]]>
export type Slow = Build<0>
