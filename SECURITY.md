## TypeScript Security Properties

The TypeScript project documents the [Security Properties of `tsc`](https://github.com/microsoft/TypeScript/wiki/tsc-Security-Properties). `tsrs` follows the same model; that guidance is reproduced below using the `tsrs` command name.

### Overview

The TypeScript compiler (`tsrs`) is a **build tool**, not a sandbox. It transforms TypeScript source files into JavaScript output files. This document describes what `tsrs` guarantees and does not guarantee when invoked on untrusted input.

### Security Guarantees

**No arbitrary code execution.** Running `tsrs` on a malicious `.ts` or `tsconfig.json` file will never cause the input code to be executed. The compiler parses, type-checks, and emits; it does not evaluate the programs it compiles. There is no `eval`-at-compile-time, no macro system, and no plugin mechanism that runs author-supplied code during compilation. This is the core security property of `tsrs`.

*Exception*: Enabling content mappers with `--runExternalCode` does enable execution of third-party code. Only pass this flag if you have validated which content mappers are available and that you are OK with running them.

**Deterministic side effects.** The only side effect of a successful `tsrs` invocation is writing output files (`.js`, `.d.ts`, `.map`, `.tsbuildinfo`) to disk. It does not make network requests, spawn child processes, or interact with the system beyond file I/O.

**Safe exit.** Certain adversarial inputs may cause crashes, but these crashes will unwind the process normally, and will not be a source of buffer overrun or other memory safety exploit vectors.

### Non-Guarantees

**Arbitrary file writes.** `tsrs` writes compiler output to paths derived from its configuration (`outDir`, `outFile`, `declarationDir`, etc.) and the structure of the input project. A malicious `tsconfig.json` can direct output to any path writable by the calling user. This is by design: writing files to disk *is the point* of a compiler. Callers who need to constrain output locations must do so externally (e.g., filesystem permissions, containers, sandboxing). Similarly, running `tsrs --build --clean` may delete files from disk; crafted `.tsbuildinfo` or `tsconfig.json` files may cause any file to be deleted.

**File read sandboxing.** `tsrs` will read files from paths specified in input files, and transitive references from there, including import paths and reference directives. An attacker in control of these files may therefore cause a *read* of any file path the `tsrs` process has privileges to read, and `tsrs` may reprint certain file contents in its output messages. In other words, for example, it is not generally safe to run `tsrs` on untrusted code and print back the error message contents to an untrusted party, as this could expose local filesystem contents to the attacker. Use of standard sandboxing strategies is recommended to secure scenarios similar to this.

**Resource consumption.** TypeScript's type system is Turing-complete. A crafted input file can cause `tsrs` to consume unbounded CPU time or memory during type-checking. The compiler provides no built-in timeouts or memory limits. Callers operating on untrusted input should enforce resource limits externally (e.g., `ulimit`, cgroups, process timeouts). You should not assume that an adversarially constructed program will successfully type-check in any bounded amount of time.

**Crash safety.** `tsrs` may gracefully crash, hang, or produce unexpected diagnostics when given adversarial input. While most crashes are treated as bugs and fixed when reported, the compiler does *not* guarantee graceful handling of all possible malformed inputs (e.g. an unbounded series of `f(f(f(f(...`).

### Language Service

The TypeScript Language Service only executes in the context of a [trusted workspace (VS Code)](https://code.visualstudio.com/docs/editing/workspaces/workspace-trust) or [trusted folder (Visual Studio)](https://learn.microsoft.com/en-us/visualstudio/ide/trust-settings?view=visualstudio). Similar to the compiler, there are no guaranteed resource caps in the Language Service, and hangs may occur in the presence of adversarial inputs.

### Summary

| Property | Guaranteed? |
|---|---|
| No execution of input code | ✅ Yes |
| Side effects limited to file writes | ✅ Yes |
| Output written only to expected paths | ❌ No — controlled by config |
| Bounded time and memory | ❌ No — type system is Turing-complete |
| No crashes on adversarial input | ❌ No |

**In short:** `tsrs` is safe to run on untrusted code in the sense that it will only read and write files, and never execute the code it compiles or other arbitrary code. All other resource and path constraints are the caller's responsibility.

Specifically, **crashes, stack overflows, nonlinear performance, and hangs** are generally not considered vulnerabilities unless they result in arbitrary code execution, and should be reported through the [normal issue flow](https://github.com/maschwenk/tsrs/issues/new/choose) instead.

## Security

We take the security of this project seriously.

If you believe you have found a security vulnerability in this repository, please report it to us as described below.

## Reporting Security Issues

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please report them privately through [GitHub Security Advisories](https://github.com/maschwenk/tsrs/security/advisories/new).

Please include the requested information listed below (as much as you can provide) to help us better understand the nature and scope of the possible issue:

  * Type of issue (e.g. buffer overflow, SQL injection, cross-site scripting, etc.)
  * Full paths of source file(s) related to the manifestation of the issue
  * The location of the affected source code (tag/branch/commit or direct URL)
  * Any special configuration required to reproduce the issue
  * Step-by-step instructions to reproduce the issue
  * Proof-of-concept or exploit code (if possible)
  * Impact of the issue, including how an attacker might exploit the issue

This information will help us triage your report more quickly.

## Preferred Languages

We prefer all communications to be in English.

## Policy

We follow the principle of Coordinated Vulnerability Disclosure.
