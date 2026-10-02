export function greet(name: string): string { return "hi " + name; }
export const PI = 3.14;
export interface Shape { area(): number; }
export class Circle implements Shape {
    constructor(public r: number) {}
    area() { return PI * this.r * this.r; }
}
export default class Base {
    run() { greet("x"); if (this) { return 1; } throw new Error(); }
}
