export class Animal {
    private secret = 1;
    static count = 0;
    constructor(protected readonly kind: string) { Animal.count++; }
    speak(): string { return this.kind + this.secret; }
    static make() { return new this("x"); }
}
export class Dog extends Animal {
    constructor() { super("dog"); }
    speak() { return "woof" + super.speak(); }
}
function helper(x: number) { return x * 2; }
export { helper, helper as twice };
export default function () { return 1; }
type Mode = "on" | "off";
export function setMode(m: Mode) { return m === "on"; }
export namespace NS { export const v = 1; export function f() { return v; } }
