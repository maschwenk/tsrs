// An immediately invoked function's arguments are evaluated before its body; a loop's back edge reaches nodes that
// start after the reference.
declare function cond(): boolean;
export function iife(x: string | number) {
    let y: string | number = x;
    ((n: number) => {
        const s: string = y;
    })(y = 1);
}

export function loop(x: string | number, xs: number[]) {
    let y: string | number = x;
    for (const v of xs) {
        const s: string = y;
        y = v;
    }
}

export function doWhile(x: string | number) {
    let y: string | number = "a";
    do {
        const s: string = y;
        if (cond()) y = 1;
    } while (cond());
}
