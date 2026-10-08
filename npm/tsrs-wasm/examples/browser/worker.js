// The checker recurses deeply: run it in a worker, whose stack the browser makes larger than the page's. Safari's
// worker stacks are smaller than Chrome's and Firefox's, so very deep expressions can overflow there first.
import { tsc } from "../../browser.js";

self.onmessage = async ({ data }) => {
    try {
        const start = performance.now();
        const r = await tsc(["-p", "/"], {
            cwd: "/",
            files: {
                "/tsconfig.json": JSON.stringify({ compilerOptions: { strict: true, noEmit: true, target: "es2022", lib: ["es2022"] }, files: ["src/main.ts"] }),
                "/src/main.ts": data.source,
            },
        });
        self.postMessage({ ...r, ms: performance.now() - start });
    } catch (e) {
        self.postMessage({ error: String(e?.stack ?? e) });
    }
};
