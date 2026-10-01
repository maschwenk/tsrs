# Public release audit

Audit of the repository before it is made public (2026-10-01, `main` at the commits that added this file). It covers
secrets, private data, legal files, GitHub Actions and repository settings.

## Secrets: none found

- `trufflehog git` over every commit on every ref (404 commits): 0 findings, verified or unverified.
- Regex scan of the full `git log -p --all` for npm tokens (`npm_…`), GitHub tokens (`ghp_`/`gho_`/`ghu_`/`ghs_`/`ghr_`),
  AWS keys (`AKIA…`), private keys (`-----BEGIN … PRIVATE KEY`), Slack tokens (`xox?-`), `sk-…` API keys, URLs with
  embedded credentials, `_authToken=` and long `Bearer` tokens, plus `key/secret/password/token = <16+ chars>`
  assignments: 0 real hits (only Rust identifiers such as `question_token` and a `password` field in `lib.dom.d.ts`).
- No `.npmrc`, `.env`, key or credential file was ever tracked. The only secret is the `NPM_TOKEN` repository secret,
  which lives in GitHub settings, not in git.

## Private data inventory

### (a) Safe to publish as is

| item | why |
| --- | --- |
| `crates/`, `crates/tsrs_vfs/libs/*.d.ts` | The port and TypeScript's own lib files (Apache-2.0). |
| `tools/oracle/**` | Go oracle programs built against the TypeScript Go sources. |
| `testdata/regressions/missing-props-head-suppression` | Synthetic. Models MikroORM's public `Reference`/`EntityRef` types (MIT) with an invented class `Loc`; no private source. Re-run at HEAD: tsrs output equals `expected.txt` (from `tsgo-ref`). |
| `testdata/regressions/unique-symbol-name-truncation` | Synthetic (well-known symbols on an object literal). Re-run at HEAD: equals `expected.txt`. |
| `bench/**` | Public projects only (microsoft/typescript-benchmarking: vscode, webpack, xstate, mui-docs, Compiler). |
| `upstream/patches`, `upstream/results/*.jsonl`, `upstream/pr-*.md` | Patches against microsoft/TypeScript and counter measurements; no paths or names. |
| Measured numbers in `docs/STATUS.md`, `notes/`, `upstream/README.md`, code comments | Kept, as required. They describe an unnamed project. |
| Mentions of public libraries the private project uses (zod, MikroORM, vitest, mongoose, Temporal) | Generic; also used by the mutation-testing heuristics in `tools/mutate/`. |
| "Max", "Max's Mac" (README benchmark header, upstream PR drafts) | The maintainer's name and benchmark hardware. |
| `deny.toml`, `Cargo.lock` | `cargo deny check` passes (advisories, licenses, bans, sources); `cargo audit`: 0 vulnerabilities in 51 crates. |

### (b) Scrubbed

| item | change |
| --- | --- |
| Absolute local paths (`/Users/<user>/Developer/tsrs-work/...`) in `docs/DEBUGGING.md`, `docs/PORTING.md`, `docs/BODY_PORTING.md`, `notes/decl-helper-brief.md`, `upstream/README.md`, `tools/project-types-compare.py` | `$TSRS_WORK/...` placeholder, defined at the top of `docs/DEBUGGING.md`. |
| Path of the private monorepo checkout (in DEBUGGING.md, `tools/mutate/mutate.py`, `upstream/tools/measure.py`) | `$PRIVATE_PROJECT` in docs; in code, required environment variables with no default (`MUT_CLONE`, `MUT_PRISTINE`, `PROJECT`). |
| Private defaults in `tools/project-types-packages.sh`, `upstream/tools/testfiles.sh`, `tools/mutate/mutate.py` (`MUT_REF`) | `${TSRS_WORK:-~/tsrs-work}`-relative defaults or required variables. |
| The private project's name (≈190 occurrences in docs, notes, upstream README, code comments, tool names) | "a 38k-file private TypeScript monorepo (5.9M lines)" at first mention, "the private monorepo" after. |
| Tool and note files named after the private project | Renamed to `tools/project-types-*`, `tools/oracle/project-types`, `notes/fix-mutation.md`, `notes/fix-project-types.md`. |
| The committed `--sample` list: 2,000 file paths of the private project | Deleted. `tools/project-types-sample.py` regenerates it into `target/`. |
| Internal file names in notes (`notes/fix-mutation.md`, `notes/mem-assignment.md`, `notes/mem-round3.md`) | Described generically ("a collection-service module", "a test-database helper module imported by 3,215 files"). Numbers kept. |
| `npm/README.md` "Adopting it in the ... monorepo" (named the private repository, its app, its scripts and dependency versions) | Replaced with generic pnpm-workspace steps. |
| Agent briefs saying the shell's cwd is "an unrelated monorepo" | "an unrelated repository". |
| A tracked `tools/oracle/ast/__pycache__/*.pyc` | Untracked; `*.pyc` ignored. |
| `bench/results/*.json` recorded the home directory in the tsgo binary path | `bench/run.py` writes `~` instead; existing result fixed. |

Verification: `git grep -i` for the project name, the monorepo checkout path, `/Users/`, and the internal file names
returns nothing in the working tree.

### (c) Needs Max's decision

1. **Git history still contains everything in (b).** 85 commit messages name the private project; earlier versions
   of the files above contain the local paths including the private monorepo checkout path, the 2,000 private file
   paths (the deleted `tools/*-types-sample.txt`, 2 commits), internal file names, and the old npm README section. Tags
   `body-base`, `nb-base`, `v0.1.0`, `v0.1.1` and the branches `chore/depot-ci-migration` and `probe/depot` point
   into that history. Making the repository public publishes all of it. Options:
   - Publish squashed history: a single root commit of the current tree, either in this repository (force-push;
     disable the ruleset first) or in a new public repository. Simplest and complete. Commit SHAs cited in
     `docs/STATUS.md` and `notes/` stop resolving.
   - Rewrite: `git filter-repo --replace-text <patterns> --replace-message <patterns> --path <deleted sample list> --invert-paths`,
     then force-push every ref and delete or recreate the tags. Keeps the history shape, rewrites every SHA, and
     needs a careful pattern list (project name, paths, file names).
   - Accept the history as is.
2. **Actions run logs and artifacts become public** with the repository (about 85 runs; 13 artifacts: bench logs,
   conformance results, release binaries and npm tarballs). They come from the repository's own code on
   GitHub-hosted runners and were not reviewed one by one. They expire after 90 days, or delete them first
   (`gh run list` / `gh run delete`, `gh api -X DELETE repos/maschwenk/tsrs/actions/artifacts/<id>`).
3. **Counters of the private project** (37,942 files, 5,941,654 lines, 25,973,354 symbols, ...) stay in the docs. They
   do not name the project, but someone with access to it could match them.
4. **Agent-workflow docs** (`docs/DEBUGGING.md`, `docs/BODY_PORTING.md`, `notes/*-brief.md`) are written as
   instructions to AI agents (worktrees under `$TSRS_WORK`, landing directly on `main`). Keep, trim, or move.
5. **`bench/results/` growth**: each push to `main` adds ~50 KB of JSON plus ~2 KB of Markdown. Keep only the last N
   (e.g. 20) runs, or move results to a separate branch. Not changed here (owned by the bench workflow).
6. **`NOTICE` copyright line** "Copyright 2026 Max Schwenk and the tsrs contributors": confirm the wording.
7. **Code of Conduct contact**: `CODE_OF_CONDUCT.md` points to @maschwenk; add a contact address if wanted.
8. **Commit author email** `maschwenk@gmail.com` is on every commit (visible once public).

## Legal files

- `LICENSE`: Apache-2.0 (TypeScript's license text). Source headers are not required.
- `NOTICE` (repository root, moved from `npm/NOTICE.txt`; `npm/build.mjs` copies it into every npm package):
  derivative-work statement, TypeScript's copyright, and TypeScript's third-party notices unchanged.
- `README.md` "Provenance": derivative of microsoft/TypeScript, the Go implementation is the specification, written
  with AI coding agents directed by Max, evidence in `docs/STATUS.md`.
- `CONTRIBUTING.md`, `SECURITY.md` (private advisories), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1),
  `.github/CODEOWNERS`, issue forms, PR template.

## GitHub Actions

- Every workflow has a top-level `permissions: contents: read`. Elevations: `bench.yml` bench job `contents: write`
  (pushes the results commit), its watchdog `actions: write` (cancels a stuck run); `release.yml` publish job
  `id-token: write` (npm provenance). `NPM_TOKEN` is referenced only in the publish step.
- All actions pinned to full commit SHAs with the version in a comment; Dependabot (`.github/dependabot.yml`) updates
  `github-actions` and `cargo` weekly, grouped. Note: Dependabot does not scan `.depot/workflows/`.
- No `pull_request_target`. Fork pull requests run `ci.yml` on GitHub-hosted runners without secrets, after approval.
  The Depot CI job (`.depot/workflows/ci.yml`) skips fork pull requests with
  `if: github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository`
  (the plain `head.repo.full_name == github.repository` test would also skip every push). `bench.yml` never runs on
  pull requests and only in `maschwenk/tsrs`.
- Concurrency groups on CI, Bench and Release (release never cancels in progress). `release.yml`'s only dispatch input
  defaults to `dry_run: true`.
- Bench commit-back loop guards: pushed with `GITHUB_TOKEN` (pushes made with it do not trigger workflows), touches
  only `README.md` and `bench/results/**` (both in `paths-ignore`), and carries `[skip ci]`.
- `actions/checkout` uses `persist-credentials: false` wherever the job does not push.
- `actionlint` is clean (`.github/actionlint.yaml` declares the Depot runner labels).

## Repository settings applied (2026-10-01)

| setting | applied | revert |
| --- | --- | --- |
| Wiki, Projects | disabled | `gh api -X PATCH repos/maschwenk/tsrs -F has_wiki=true -F has_projects=true` |
| Delete branch on merge | on | `gh api -X PATCH repos/maschwenk/tsrs -F delete_branch_on_merge=false` |
| Dependabot alerts | on | `gh api -X DELETE repos/maschwenk/tsrs/vulnerability-alerts` |
| Dependabot security updates | on | `gh api -X DELETE repos/maschwenk/tsrs/automated-security-fixes` |
| Default workflow token | read-only, cannot approve PRs (was already so) | `gh api -X PUT repos/maschwenk/tsrs/actions/permissions/workflow -f default_workflow_permissions=write` |
| Ruleset "main: no force-push, no deletion" (id 24309355) | blocks force pushes and deletion of the default branch; no PR or status-check requirement, no bypass | `gh api -X DELETE repos/maschwenk/tsrs/rulesets/24309355` |

Not available while the repository is private (enable after the visibility change, commands in the checklist below):
secret scanning, push protection, and the fork pull request approval policy.

## Checklist for Max

1. Decide on the git history (item 1 of (c)) before anything else; a public repository cannot be un-published.
2. Make the repository public:
   `gh repo edit maschwenk/tsrs --visibility public --accept-visibility-change-consequences`
3. Then enable what private repositories do not offer:
   ```sh
   gh api -X PATCH repos/maschwenk/tsrs --input - <<< '{"security_and_analysis":{"secret_scanning":{"status":"enabled"},"secret_scanning_push_protection":{"status":"enabled"}}}'
   gh api -X PUT repos/maschwenk/tsrs/actions/permissions/fork-pr-contributor-approval -f approval_policy=all_external_contributors
   ```
4. Require pull requests and green CI on `main` (only when agents stop landing directly on `main`):
   ```sh
   gh api -X PUT repos/maschwenk/tsrs/rulesets/24309355 --input - <<'JSON'
   {"name":"main: PR + CI, no force-push, no deletion","target":"branch","enforcement":"active",
    "conditions":{"ref_name":{"include":["~DEFAULT_BRANCH"],"exclude":[]}},
    "rules":[{"type":"non_fast_forward"},{"type":"deletion"},
     {"type":"pull_request","parameters":{"required_approving_review_count":0,"dismiss_stale_reviews_on_push":false,
      "require_code_owner_review":false,"require_last_push_approval":false,"required_review_thread_resolution":false}},
     {"type":"required_status_checks","parameters":{"strict_required_status_checks_policy":false,
      "required_status_checks":[{"context":"check-and-test","integration_id":15368}]}}],
    "bypass_actors":[{"actor_id":5,"actor_type":"RepositoryRole","bypass_mode":"always"}]}
   JSON
   ```
   (`bypass_actors` lets repository admins push directly; drop it to hold admins to the same rule. The bench
   workflow's results push from `GITHUB_TOKEN` would then need a bypass or a separate results branch.)
5. npm: `release.yml` adds `--provenance` automatically once the repository is public. Consider npm trusted publishing
   (OIDC) to retire the long-lived `NPM_TOKEN`, and a protected `npm` environment with a required reviewer on the
   publish job.
6. Optionally require SHA-pinned actions for every workflow (all are pinned now; temporary branches such as
   `probe/depot` would fail):
   `gh api -X PUT repos/maschwenk/tsrs/actions/permissions -F enabled=true -f allowed_actions=all -F sha_pinning_required=true`
7. Items 2-8 of (c): Actions logs and artifacts, private-project counters, agent docs, bench results growth, NOTICE
   wording, CoC contact, author email.
