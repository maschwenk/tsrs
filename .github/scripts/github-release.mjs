#!/usr/bin/env node
// Called by release.yml: validate the release before building, then announce it after npm publication.
import { execFileSync } from "node:child_process";
import fs from "node:fs";

export function validateRelease({ crateVersion, packageVersion, refType, tag, dryRun, notes }) {
    if (refType === "tag") {
        if (tag !== `v${crateVersion}` && tag !== `v${packageVersion}`) {
            throw new Error(`tag ${tag} does not match ${crateVersion} or ${packageVersion}`);
        }
    } else if (!dryRun) {
        throw new Error("Real releases require a version tag; dispatch on an existing tag or set dry_run=true.");
    }
    if (!notes.trim()) throw new Error(`release notes for ${crateVersion} are empty`);
}

export function publishGitHubRelease(config, gh = args => execFileSync("gh", args, { encoding: "utf8" })) {
    validateRelease(config);
    if (config.dryRun) return "Dry run: GitHub release skipped.";
    const { crateVersion, tag, repository, sha, notes, notesFile } = config;
    if (!repository || !sha) throw new Error("GITHUB_REPOSITORY and GITHUB_SHA are required");
    const commit = JSON.parse(gh(["api", `repos/${repository}/commits/${encodeURIComponent(tag)}`]));
    if (commit.sha !== sha) throw new Error(`tag ${tag} no longer points to the workflow commit ${sha}`);

    let existing;
    try {
        existing = JSON.parse(gh(["api", `repos/${repository}/releases/tags/${encodeURIComponent(tag)}`]));
    } catch (error) {
        // Authentication, transport and server errors must not be mistaken for an absent release.
        if (!String(error.stderr).includes("(HTTP 404)")) throw error;
    }
    const title = `TSRS ${crateVersion}`;
    if (existing) {
        if (existing.tag_name !== tag) throw new Error("existing release has a different tag");
        if (!existing.draft) {
            if (existing.name !== title || existing.prerelease || existing.body?.trim() !== notes.trim()) {
                throw new Error(`published release ${tag} differs from the intended release metadata`);
            }
            // Do not move latest back to an older release when a completed workflow is rerun.
            return `Release ${tag} already published with matching metadata.`;
        }
        gh(["release", "edit", tag, "--repo", repository, "--title", title, "--notes-file", notesFile,
            "--draft=false", "--prerelease=false", "--latest"]);
    } else {
        gh(["release", "create", tag, "--repo", repository, "--verify-tag", "--title", title,
            "--notes-file", notesFile, "--prerelease=false", "--latest"]);
    }
    return `Published GitHub release ${tag}.`;
}

if (import.meta.main) {
    try {
        const command = process.argv[2];
        if (command !== "preflight" && command !== "publish") throw new Error("usage: github-release.mjs preflight|publish");
        if (process.env.DRY_RUN !== "true" && process.env.DRY_RUN !== "false") throw new Error("DRY_RUN must be true or false");
        const version = flag => execFileSync(process.execPath, ["npm/build.mjs", flag], { encoding: "utf8" }).trim();
        const crateVersion = version("--print-crate-version");
        const notesFile = `docs/releases/${crateVersion}.md`;
        const config = {
            crateVersion,
            packageVersion: version("--print-version"),
            notesFile,
            notes: fs.readFileSync(notesFile, "utf8"),
            refType: process.env.GITHUB_REF_TYPE,
            tag: process.env.GITHUB_REF_NAME,
            dryRun: process.env.DRY_RUN === "true",
            repository: process.env.GITHUB_REPOSITORY,
            sha: process.env.GITHUB_SHA,
        };
        validateRelease(config);
        console.log(command === "publish" ? publishGitHubRelease(config) : `Release ${crateVersion}: tag and notes validated.`);
    } catch (error) {
        console.error(`::error::${error.message}`);
        process.exitCode = 1;
    }
}
