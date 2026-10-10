function useIt() {
    if (isStr) { const n: number = g; }
}
namespace N {
    function useToo() {
        if (isNum) { const s: string = g; }
    }
}
declare function assert(value: unknown): asserts value;
function useAssert() {
    assert(isStr);
    const n: number = g;
}
