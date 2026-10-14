#!/usr/bin/env bash
# The pinned triagebot-action revision assumes these labels exist. Keep existing
# labels untouched and create only the ones the action needs but cannot bootstrap.
set -euo pipefail

existing="$(gh label list --repo "$GITHUB_REPOSITORY" --limit 200 --json name --jq '.[].name')"

while IFS=$'\t' read -r name color description; do
  if grep -Fqx -- "$name" <<<"$existing"; then
    continue
  fi

  gh label create "$name" \
    --repo "$GITHUB_REPOSITORY" \
    --color "$color" \
    --description "$description"
  existing+=$'\n'"$name"
done <<'LABELS'
triage: needs triage	fbca04	Waiting for automated investigation
triage: not actionable	ededed	Not an actionable bug report
triage: needs reproduction	d876e3	More information is needed to reproduce the report
triage: skipped	cfd3d7	Cannot be investigated in the triage environment
triage: unable to reproduce	e4e669	Automated investigation could not reproduce the report
triage: unable to fix	b60205	Reproduced, but no verified automated fix was found
triage: failed	d73a4a	Automated investigation failed unexpectedly
triage: fix pending	0e8a16	A proposed fix is waiting for reporter verification
triage: fix rejected	d93f0b	The reporter says the proposed fix does not work
triage: fix verified	0e8a16	The proposed fix was verified
fix verified	0e8a16	Pull request created from a verified automated fix
- P0	b60205	Release-blocking or broadly catastrophic
- P1	d93f0b	High-impact correctness or reliability problem
- P2	fbca04	Confirmed issue with normal priority
- P3	0e8a16	Narrow or low-impact issue
pkg: checker	5319e7	Type checking and diagnostics
pkg: parser	1d76db	Scanning, parsing, and binding
pkg: emit	c5def5	JavaScript, declaration, and source-map emit
pkg: cli	0052cc	Command-line compiler and build mode
pkg: lsp	006b75	Language server and editor features
pkg: node-api	7057ff	Node.js API and transport
pkg: wasm	bfd4f2	WebAssembly package
pkg: infrastructure	ededed	Build, packaging, CI, and repository tooling
LABELS
