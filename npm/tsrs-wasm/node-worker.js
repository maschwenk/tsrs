// Runs one tsc request in a worker thread (node.js posts the compiled module and the options).
import { parentPort, workerData } from "node:worker_threads";
import { run } from "./node-run.js";

parentPort.postMessage(run(workerData.module, workerData.options));
