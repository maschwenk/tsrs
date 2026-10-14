# TypeScript main update

Update from `b85298b6a81f772d080b0455de0ca9d744cd6fd6` to
`9eedcb0dd7257f8a1a175adbef28f3b6d3492d7a` (main on 2026-10-14).
The stack follows upstream merge order. Each row records the corresponding port
or why the change does not apply to tsrs. The source pin advances after the ports.

| upstream commit | disposition |
| --- | --- |
| [#64547](https://github.com/microsoft/TypeScript/commit/9adc871ff47f79b3e99ff2b1cbe9efb140d76af8) | TypeScript's Azure Node installation task; no tsrs change. |
| [#64545](https://github.com/microsoft/TypeScript/commit/697b847e11e91dfbff79f1d189b802b751999f03) | Port export modifiers on both alias and renamed expando exports. |
| [#64482](https://github.com/microsoft/TypeScript/commit/81d3f3f5e89786acc33d1881a0864bb0bd68068b) | Port expression-context classification for named functions and classes. |
| [#64461](https://github.com/microsoft/TypeScript/commit/834e7862c031a3f5f24c53acf51056a4f558c3e4) | Port cyclic-structure placeholders and declaration truncation tracking. |
| [#64525](https://github.com/microsoft/TypeScript/commit/619d485a639c83a2d82860b5797db4984cecfc91) | Port the guard against contextual typing by the owning class. |
| [#64243](https://github.com/microsoft/TypeScript/commit/792ffccb90f548cd3bdd7fc0b44f0ca9e1b169a7) | Port string-literal-only module import attribute annotations. |
| [#64530](https://github.com/microsoft/TypeScript/commit/c4d731aae17e83b73f48ece1dcb2d42b02179da0) | Port constraint filtering for pure return inference in recursive calls. |
| [#64559](https://github.com/microsoft/TypeScript/commit/79feed291a5c6c1d42ec040c4b42e7c2e9a1e2c4) | Azure VSIX release templates; tsrs has no corresponding pipeline. |
| [#64522](https://github.com/microsoft/TypeScript/commit/64c5fa0b92ade9b5f14db616d351ffe1990ca7fd) | Upstream customConditions fixture correction; consumed from the new pinned corpus. |
| [#64555](https://github.com/microsoft/TypeScript/commit/5f5dee5b610526cf3c597457cca336fde65e5f11) | VSIX signing command; tsrs has no corresponding release artifact. |
| [#64553](https://github.com/microsoft/TypeScript/commit/c8e9b7d259268e30605ed14910cb8d118929ed63) | Port invokeOnce for alias/reference arguments and its priority/variance key. |
| [#64554](https://github.com/microsoft/TypeScript/commit/43521c80ec5ebe4bc2948f1df3747773e48d8312) | Port snapshot preferences/import preparation and require same-snapshot completion symbols; retain equivalent flat Rust preference storage. SDK refresh is included with the final pin. |
| [#64558](https://github.com/microsoft/TypeScript/commit/edf7da4e93ab65e1f7766deae156c35132c0cde3) | Port reverse-mapped emit expansion, cycle diagnostics, and declaration-based depth limits. |
| [#64574](https://github.com/microsoft/TypeScript/commit/02c1c9a6ab49334a5ca9c1d7852647b7a3fad18a) | Port the empty-statement fallback after CommonJS hoisting. |
| [#64566](https://github.com/microsoft/TypeScript/commit/237b14a5d98f8e8f748ff36002fdd99786230d90) | Port first-declaration interface/enum diagnostics and once-per-checker merged export checks, replacing the old scheduling workaround. |
| [#64571](https://github.com/microsoft/TypeScript/commit/688d86d7f647781565dc1f6438a4fb09b5179160) | Port getSymbolOfDeclaration with exact cached-AST ownership and index validation; SDK refresh is included with the final pin. |
| [#64457](https://github.com/microsoft/TypeScript/commit/e8fd69fc1e1974f7f7410f79a04adad692dff0dc) | Port generated declarations, metadata, runner variation lists and option change comparisons, legacy watch-option removal, validation, and ordered option equality; Rust retains its existing typed option fields. |
| [#64556](https://github.com/microsoft/TypeScript/commit/15dee00a1c6549a4953ab17ccac4c56d252ba4cc) | Port canonical container cycle identities and inferred-parameter nodebuilder cache keys. |
| [#64573](https://github.com/microsoft/TypeScript/commit/af36d532b0120413a8b6d2157a7f5be7f8d61f03) | Port cyclic alias-chain resolution for export-equals class visibility. |
| [#64093](https://github.com/microsoft/TypeScript/commit/bf00f213ffd67ce4cc765c0db341e86c7057be44) | Copy upstream Promise.allKeyed/allSettledKeyed declarations, library references and load order. |
| [#64577](https://github.com/microsoft/TypeScript/commit/82f054616325c1550b184f26a64f26b44ce6b99d) | Upstream security contact documentation; tsrs maintains its own policy. |
| [#64544](https://github.com/microsoft/TypeScript/commit/59f5b0233f9d2411aacb77c7342ddb31002dd3dd) | Port dynamic URI encoding, resolution, symlink spelling, source-map mapping/serialization, watcher paths, API transactions, and shutdown synchronization. ATA and attached LSP API sessions remain outside the documented port scope; SDK refresh is included with the final pin. |
| [#64578](https://github.com/microsoft/TypeScript/commit/09b1db0617311f3b39f165bb3480c0ce2f718b27) | Port verbatim-module erasure while retaining explicitly written empty import lists. |
| [#63915](https://github.com/microsoft/TypeScript/commit/2f9fd09a104ef14268944a0aeb9cdc4d80abb6a6) | Port source-phase syntax, checking, resolution, emit, API codec, language-service import handling and bundled module-source declarations; all 31 source-phase cases match. |
