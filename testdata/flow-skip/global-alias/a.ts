// A script's top-level const is visible in another file: a condition there may inline it.
declare const g: string | number;
const isStr = typeof g === "string";
namespace N {
    export const isNum = typeof g === "number";
}
