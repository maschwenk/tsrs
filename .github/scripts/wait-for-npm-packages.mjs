#!/usr/bin/env node
// Based on oxc's .github/scripts/wait-for-npm-packages.mjs; read the staged main package's optional dependencies.

import fs from "node:fs";

const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

async function waitForPackage([name, version]) {
    const spec = `${name}@${version}`;
    const metadataUrl = `https://registry.npmjs.org/${encodeURIComponent(name)}/${encodeURIComponent(version)}`;
    // Leave time for the remaining publishes within the release job's 30-minute timeout.
    const deadline = Date.now() + 10 * 60 * 1000;

    console.log(`Waiting for ${spec} to be available.`);
    while (Date.now() < deadline) {
        try {
            const metadata = await fetch(metadataUrl, {
                headers: { "cache-control": "no-cache" },
                signal: AbortSignal.timeout(30_000),
            });
            if (!metadata.ok) throw new Error();

            const tarball = await fetch((await metadata.json()).dist.tarball, {
                headers: { "cache-control": "no-cache", range: "bytes=0-0" },
                signal: AbortSignal.timeout(30_000),
            });
            await tarball.body?.cancel();
            if (!tarball.ok) throw new Error();

            console.log(`✓ ${spec} is available.`);
            return;
        } catch {
            await sleep(10_000);
        }
    }

    throw new Error(`Timed out waiting for ${spec}.`);
}

const { optionalDependencies } = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
Promise.all(Object.entries(optionalDependencies).map(waitForPackage)).catch((error) => {
    console.error(`::error::${error.message}`);
    process.exit(1);
});
