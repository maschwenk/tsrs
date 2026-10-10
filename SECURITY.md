# Security policy

Please report security vulnerabilities privately through GitHub: open the repository's **Security** tab and choose
**Report a vulnerability** (a private security advisory). Do not open a public issue for a vulnerability.

Include the affected version (`tsrs --version`), a description, and a reproduction if you have one. You should get a
first response within a week.

tsrs is a type checker: it reads TypeScript source and configuration files and does not execute them, except that
with `--runExternalCode` it starts the content-mapper commands named in a tsconfig's `contentMappers`, as tsc does.
Reports that are in scope include memory-safety problems, crashes or unbounded resource use triggered by input files,
path handling that reads outside the project, and problems in the published npm packages or the release workflow.

Only the latest release and `main` receive fixes.
