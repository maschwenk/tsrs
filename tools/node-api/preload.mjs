// Loaded with `--import` into every node:test file process. While a test runs, process.env.NODE_API_TEST holds
// the test's full name; API server processes spawned meanwhile inherit it and proxy.mjs stamps it into the trace,
// letting the inventory attribute each wire method to the upstream tests that sent it.
//
// Root-level beforeEach applies to every test in the process without wrapping test()/it(), so node:test keeps the
// real call-site file and line for reporting. Suite-level before/after hooks are wrapped (they produce no test
// events) so servers spawned there are attributed to "<suite> (before)".

import module from "node:module";

const require = module.createRequire(import.meta.url);
const nodeTest = require("node:test");

nodeTest.beforeEach(ctx => {
    const name = ctx?.fullName ?? ctx?.name;
    if (name !== undefined) process.env.NODE_API_TEST = name;
});

function wrapHook(orig, label) {
    return function(fn, ...rest) {
        if (typeof fn !== "function") return orig.call(this, fn, ...rest);
        const wrapped = function(ctx, ...args) {
            const name = ctx?.fullName ?? ctx?.name;
            if (name !== undefined) process.env.NODE_API_TEST = `${name} (${label})`;
            return fn.call(this, ctx, ...args);
        };
        Object.defineProperty(wrapped, "length", { value: fn.length });
        return orig.call(this, wrapped, ...rest);
    };
}

for (const key of ["before", "after"]) nodeTest[key] = wrapHook(nodeTest[key], key);
module.syncBuiltinESMExports();
