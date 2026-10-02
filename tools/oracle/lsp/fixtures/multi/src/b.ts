import { makePoint, origin, type Point } from "./a";
export function translate(p: Point, dx: number): Point {
    return makePoint(p.x + dx, p.y);
}
export const moved = translate(origin, 3);
