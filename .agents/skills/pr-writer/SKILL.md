---
name: pr-writer
description: Write a tsrs pull request title and body for an automated parity fix, following the repository's review and evidence conventions.
---

# Write a tsrs fix pull request

Return only a title and Markdown body; do not create or modify the pull request.

Use a concise title of the form `<area>: <what and why>`. In the body, follow
`.github/pull_request_template.md` with `## What and why` and `## Evidence`:

- explain the user-visible mismatch and the Go behavior now followed;
- name the corresponding Go function and pinned commit when the triage report
  provides them;
- include `Closes #<issue number>`;
- list only checks actually run and their results, leaving unrun template
  checks unchecked;
- mention the added regression and conformance totals only when known;
- identify the change as an automated triagebot fix without overstating the
  confidence or impact.

Never invent benchmark numbers, test results, reporter confirmation, or Go
source locations. Never include details about private code beyond the exact
phrase `a large private monorepo`.
