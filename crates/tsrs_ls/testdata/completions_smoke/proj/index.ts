import { alpha } from "./other";
import {  } from "./other";
interface Point {
    /** The x coordinate. */
    x: number;
    y: number;
    label?: string;
    /** @deprecated use x */
    oldX: number;
    move(dx: number): void;
}
declare const p: Point;
p.x;
const q: Point = { x: 1, };
declare const maybe: Point | undefined;
maybe.y;
enum Color { Red, Green = "green" }
Color.Red;
namespace NS { export const inner = 1; export type T = string; }
NS.inner;
let kind: "small" | "large" = "small";
kind = "large";
const rec: { "a b": number; c: number } = { "a b": 1, c: 2 };
rec["c"];
let typed: Poi;
class Shape {
    private secret = 1;
    area(): number { return this.secret; }
    describe() {
        return this.area();
    }
}
class Circle extends Shape {
    radius = 1;
    
}
function labels() {
    outer: for (;;) {
        break outer;
    }
}
/**
 * @par
 */
function documented(first: number, second: string) {}

function needsDoc(a: number) {}
import * as other from "./other";
other.beta;
function call(mode: "on" | "off") {}
call("on");
let tuple: [number, string] = [1, ""];
const s = "text";
s.length;
