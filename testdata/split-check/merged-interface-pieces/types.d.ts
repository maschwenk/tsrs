interface Left { kind: "left"; shared: string }
interface Right { kind: "right"; shared: string }
interface Merged extends Left {
    a: number;
}
declare const one: number;
declare const two: string;
declare function three(x: number): string;
declare const four: boolean;
interface Merged extends Right {
    b: number;
}
declare const five: Merged;
