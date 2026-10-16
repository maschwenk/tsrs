import assert from "node:assert/strict";
import { test } from "node:test";
import { checkPackage, releasePackages } from "./wait-for-npm-packages.mjs";

const spec = ["@tsrs/wasm", "0.16.0-ts7.1.0-dev.20260929"];

function registry({ metadata = true, tarball = true, latest = spec[1], version = spec[1] } = {}) {
    const urls = [];
    let canceled = false;
    return {
        urls,
        get canceled() { return canceled; },
        async fetch(url) {
            urls.push(url);
            if (url.endsWith("/latest")) return { ok: true, json: async () => ({ version: latest }) };
            if (url === "https://example.invalid/package.tgz") {
                return { ok: tarball, body: { cancel: async () => { canceled = true; } } };
            }
            return { ok: metadata, json: async () => ({ name: spec[0], version, dist: { tarball: "https://example.invalid/package.tgz" } }) };
        },
    };
}

test("release verification requires both the tarball and matching latest tag", async () => {
    const mock = registry();
    await checkPackage(spec, true, mock.fetch);
    assert.equal(mock.urls.length, 3);
    assert.ok(mock.canceled);
    assert.ok(mock.urls[0].includes("%40tsrs%2Fwasm"));
});

test("optional dependency propagation does not require a latest tag", async () => {
    const mock = registry({ latest: "old" });
    await checkPackage(spec, false, mock.fetch);
    assert.equal(mock.urls.length, 2);
});

for (const options of [{ metadata: false }, { tarball: false }, { latest: "old" }, { version: "wrong" }]) {
    test(`an incomplete publication is rejected: ${Object.keys(options)[0]}`, async () => {
        await assert.rejects(checkPackage(spec, true, registry(options).fetch));
    });
}

test("both manifests contribute all seven packages at a single version", () => {
    const names = ["tsrs", "@tsrs/darwin-arm64", "@tsrs/linux-x64-gnu", "@tsrs/linux-arm64-gnu", "@tsrs/linux-x64-musl", "@tsrs/linux-arm64-musl"];
    const native = { version: spec[1], packages: names.map(name => ({ name })) };
    const wasm = { version: spec[1], packages: [{ name: spec[0] }] };
    assert.deepEqual(releasePackages([native, wasm]), [...names, spec[0]].map(name => [name, spec[1]]));
    assert.throws(() => releasePackages([native, { ...wasm, version: "wrong" }]), /versions differ/);
    assert.throws(() => releasePackages([]), /must have a version/);
    assert.throws(() => releasePackages([{ ...native, packages: [] }]), /no packages/);
});
