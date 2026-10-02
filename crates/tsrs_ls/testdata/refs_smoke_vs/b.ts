import Base, { greet, PI as pi, Circle } from "./a";
import * as A from "./a";
const name = "bob";
const obj = { name, pi };
const { name: n2 } = obj;
function main() {
    greet(name);
    A.greet(n2);
    const c = new Circle(pi);
    c.area();
    new Base().run();
    label: for (;;) { break label; }
}
main();
class Sq extends Circle { area() { return super.area(); } }
const shapes: A.Shape[] = [new Circle(1), new Sq(2)];
if (shapes.length) { } else if (pi) { } else { }
const f = () => main();
