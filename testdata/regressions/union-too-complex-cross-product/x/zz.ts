import type { GetModel, Key, ModelMap } from "../map"
declare function lookup(key: Key): ModelMap[Key] | undefined
import { getA } from "./a"
export const again = getA
export const getB: GetModel = key => lookup(key)
