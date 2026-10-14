// Module runs for diff.mjs, in a worker thread with a large stack. With `census` (the stack pointer's initial
// value), the shadow stack [1024, census) is filled with 0xA5 before the run and scanned after it: the lowest byte
// that changed is the deepest the stack went.
import { parentPort, workerData } from "node:worker_threads";
import { run } from "../../npm/tsrs-wasm/node-run.js";

const FILL = 0xa5;
const LOW = 1024;

// One message per run: { options, census }; the reply is the run's result. The worker serves its slot's runs one
// at a time until diff.mjs ends it.
parentPort.on("message", ({ options, census }) => {
    let stackBytes = 0;
    const hooks = census
        ? {
              beforeRun(memory) {
                  new Uint8Array(memory.buffer, LOW, census - LOW).fill(FILL);
              },
              afterRun(memory) {
                  const b = new Uint8Array(memory.buffer, 0, census);
                  let i = LOW;
                  while (i < census && b[i] === FILL) i++;
                  stackBytes = census - i;
              },
          }
        : {};
    const result = run(workerData.module, options, hooks);
    parentPort.postMessage({ ...result, stackBytes });
});
