#!/usr/bin/env node
// `tsrs-wasm [tsc options]`: tsc on the real file system through tsrs.wasm. The tty check uses tty.isatty(1), not
// process.stdout (touching process.stdout on a pipe makes it non-blocking for every process sharing the pipe).
import tty from "node:tty";
import { tsc } from "../node.js";

const result = await tsc(process.argv.slice(2), {
    env: process.env,
    tty: tty.isatty(1),
    stdout: "inherit",
    stderr: "inherit",
});
process.exitCode = result.exitCode;
