import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { publishGitHubRelease, validateRelease } from "./github-release.mjs";

const config = {
    crateVersion: "0.16.0",
    packageVersion: "0.16.0-ts7.1.0-dev.20260929",
    refType: "tag",
    tag: "v0.16.0",
    dryRun: false,
    notes: "Release notes\n",
    notesFile: "docs/releases/0.16.0.md",
    repository: "oxc-project/tsrs",
    sha: "a".repeat(40),
};

function fakeGitHub({ existing, error = "HTTP 404", sha = config.sha } = {}) {
    const writes = [];
    return {
        writes,
        gh(args) {
            if (args[0] === "api") {
                if (args[1].includes("/commits/")) return JSON.stringify({ sha });
                if (existing) return JSON.stringify(existing);
                throw Object.assign(new Error(error), { stderr: `gh: failed (${error})` });
            }
            writes.push(args);
            return "";
        },
    };
}

test("only dry runs may use branches; either supported version tag is accepted", () => {
    assert.throws(() => validateRelease({ ...config, refType: "branch" }), /require a version tag/);
    validateRelease({ ...config, refType: "branch", dryRun: true });
    validateRelease({ ...config, tag: `v${config.packageVersion}` });
    assert.throws(() => validateRelease({ ...config, tag: "v0.15.0", dryRun: true }), /does not match/);
    assert.throws(() => validateRelease({ ...config, notes: " \n" }), /notes.*empty/);
});

test("missing notes fail preflight before any publication", () => {
    const cwd = fs.mkdtempSync(path.join(os.tmpdir(), "tsrs-release-notes-"));
    try {
        fs.symlinkSync(path.resolve("npm"), path.join(cwd, "npm"), "dir");
        const result = spawnSync(process.execPath, [path.resolve(".github/scripts/github-release.mjs"), "preflight"], {
            cwd, encoding: "utf8", env: { ...process.env, DRY_RUN: "true", GITHUB_REF_TYPE: "branch" },
        });
        assert.equal(result.status, 1);
        assert.match(result.stderr, /ENOENT.*docs\/releases\//);
    } finally {
        fs.rmSync(cwd, { recursive: true, force: true });
    }
});

test("dry runs never call GitHub", () => {
    publishGitHubRelease({ ...config, dryRun: true }, () => assert.fail("GitHub called in a dry run"));
});

test("new releases verify the tag and publish curated notes as latest", () => {
    const mock = fakeGitHub();
    publishGitHubRelease(config, mock.gh);
    assert.equal(mock.writes.length, 1);
    const args = mock.writes[0];
    assert.deepEqual(args.slice(0, 3), ["release", "create", config.tag]);
    assert.ok(args.includes("--verify-tag"));
    assert.ok(args.includes("--latest"));
    assert.ok(args.includes("--prerelease=false"));
    assert.equal(args[args.indexOf("--notes-file") + 1], config.notesFile);
    assert.equal(args[args.indexOf("--title") + 1], "TSRS 0.16.0");
});

test("a moved tag prevents creation", () => {
    const mock = fakeGitHub({ sha: "b".repeat(40) });
    assert.throws(() => publishGitHubRelease(config, mock.gh), /no longer points/);
    assert.equal(mock.writes.length, 0);
});

for (const error of ["HTTP 401", "HTTP 403", "HTTP 500", "connection reset"]) {
    test(`a failed lookup (${error}) is not an absent release`, () => {
        const mock = fakeGitHub({ error });
        assert.throws(() => publishGitHubRelease(config, mock.gh), { message: error });
        assert.equal(mock.writes.length, 0);
    });
}

const published = { tag_name: config.tag, name: "TSRS 0.16.0", body: config.notes, prerelease: false, draft: false };

test("rerunning a matching published release does not change latest or recreate it", () => {
    const mock = fakeGitHub({ existing: published });
    assert.match(publishGitHubRelease(config, mock.gh), /already published/);
    assert.equal(mock.writes.length, 0);
});

for (const change of [{ tag_name: "v0.15.0" }, { name: "Wrong" }, { body: "Wrong" }, { prerelease: true }]) {
    test(`conflicting published metadata is rejected: ${Object.keys(change)[0]}`, () => {
        const mock = fakeGitHub({ existing: { ...published, ...change } });
        assert.throws(() => publishGitHubRelease(config, mock.gh), /different|differs/);
        assert.equal(mock.writes.length, 0);
    });
}

test("a draft for the matching tag can be finalized", () => {
    const mock = fakeGitHub({ existing: { ...published, draft: true } });
    publishGitHubRelease(config, mock.gh);
    assert.equal(mock.writes.length, 1);
    assert.deepEqual(mock.writes[0].slice(0, 3), ["release", "edit", config.tag]);
    assert.ok(mock.writes[0].includes("--draft=false"));
});
