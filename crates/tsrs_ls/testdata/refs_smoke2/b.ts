import def, { Animal, Dog, helper, twice, setMode, NS } from "./a";
const d = new Dog();
d.speak();
const a: Animal = new Animal("cat");
helper(1) + twice(2) + def();
setMode("on");
let s: string = "on";
const point = { x: 1, y: 2 };
const { x, y: why } = point;
function useXY({ x }: { x: number }) { return x; }
useXY(point);
NS.f();
const n = NS.v;
interface HasLen { length: number }
const arr: HasLen = [1, 2];
const str: HasLen = "abc";
