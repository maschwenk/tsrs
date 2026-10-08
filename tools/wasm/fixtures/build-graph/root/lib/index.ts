import { add, type Point } from "../core/out/index.js";
export const origin: Point = { x: 0, y: 0 };
export const moved = add(origin, { x: 1, y: "2" });
