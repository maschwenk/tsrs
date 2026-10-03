// node:test reporter that writes one JSON line per finished test/suite, with its full suite path, so that two
// runs (Go oracle vs tsrs) can be compared test by test. Used by run-upstream.mjs alongside the TAP reporter.

import path from "node:path";

export default async function* jsonlReporter(source) {
    const stacks = new Map(); // file -> names by nesting level
    for await (const event of source) {
        const { type, data } = event;
        if (type === "test:start") {
            const stack = stacks.get(data.file) ?? [];
            stack[data.nesting] = data.name;
            stack.length = data.nesting + 1;
            stacks.set(data.file, stack);
            continue;
        }
        if (type !== "test:pass" && type !== "test:fail") continue;
        const stack = stacks.get(data.file) ?? [];
        const parents = stack.slice(0, data.nesting);
        const err = data.details?.error;
        const cause = err?.cause ?? err;
        yield JSON.stringify({
            file: data.file ? path.relative(process.cwd(), data.file) : undefined,
            path: [...parents, data.name].join(" > "),
            kind: data.details?.type ?? "test",
            ok: type === "test:pass",
            skip: data.skip !== undefined && data.skip !== false,
            todo: data.todo !== undefined && data.todo !== false,
            ms: Math.round(data.details?.duration_ms ?? 0),
            error: type === "test:fail" ? String(cause?.message ?? cause ?? err?.message ?? "").slice(0, 2000) : undefined,
        }) + "\n";
    }
}
