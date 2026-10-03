// Loaded with `--import` into every node:test file process. It wraps node:test's test/it and hooks so that,
// while a test (or a hook for it) runs, process.env.NODE_API_TEST holds the test's full name. API server
// processes spawned meanwhile inherit it, and proxy.mjs stamps it into the trace, letting the inventory
// attribute each wire method to the exact upstream tests that sent it. Test bodies are not otherwise touched.

import module from "node:module";

const require = module.createRequire(import.meta.url);
const nodeTest = require("node:test");

function nameOf(ctx) {
    return ctx?.fullName ?? ctx?.name;
}

function wrapBody(fn, label) {
    if (typeof fn !== "function") return fn;
    const wrapped = function(ctx, ...rest) {
        const name = nameOf(ctx);
        if (name !== undefined) process.env.NODE_API_TEST = label ? `${name} (${label})` : name;
        return fn.call(this, ctx, ...rest);
    };
    // node:test treats fn.length >= 2 as a callback-style test; preserve arity.
    Object.defineProperty(wrapped, "length", { value: fn.length });
    return wrapped;
}

function wrapTestFn(orig) {
    const wrapped = function(...args) {
        const i = args.findIndex(a => typeof a === "function");
        if (i >= 0) args[i] = wrapBody(args[i]);
        return orig.apply(this, args);
    };
    for (const key of ["skip", "todo", "only"]) {
        if (typeof orig[key] === "function") wrapped[key] = wrapTestFn(orig[key]);
    }
    return wrapped;
}

function wrapHook(orig, label) {
    return function(fn, ...rest) {
        return orig.call(this, wrapBody(fn, label), ...rest);
    };
}

for (const key of ["test", "it"]) nodeTest[key] = wrapTestFn(nodeTest[key]);
for (const key of ["before", "beforeEach", "after", "afterEach"]) nodeTest[key] = wrapHook(nodeTest[key], key);
module.syncBuiltinESMExports();
