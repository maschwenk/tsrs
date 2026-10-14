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
| [#64536](https://github.com/microsoft/TypeScript/commit/a4b1410230c7299957ec3f8f8dee4cccc55a4798) | Copy the upstream Array.at documentation for arrays and typed arrays. |
| [#64572](https://github.com/microsoft/TypeScript/commit/efbf3b5067547539c9cf8331d51943543c8bc91a) | VS Code content-mapped JSX insertion; the tsrs language server content-mapper capability remains unsupported. |
| [#64562](https://github.com/microsoft/TypeScript/commit/dca8e236a227e37183dcf1dd4e51b22571f5b558) | Upstream brace-expansion dependency; tsrs does not use this package. |
| [#64583](https://github.com/microsoft/TypeScript/commit/0ad3777160acc032a2063da0cb0edd02be8b23a9) | VS Code LSP middleware hooks; no corresponding extension in tsrs. SDK sources refresh with the final pin. |
| [#64598](https://github.com/microsoft/TypeScript/commit/d2f9dd68cfff50ca3c2fdf3152f808fd9d4ca2d6) | Port all four merged-symbol checker methods and cover declaration identity and parents across files; SDK refresh is included with the final pin. |
| [#64607](https://github.com/microsoft/TypeScript/commit/7e5d1c1c1de408086e2dd74cb42d5b9c6ba77461) | Go fsevents test retry helper; tsrs uses its existing Rust watcher checks. |
| [#64159](https://github.com/microsoft/TypeScript/commit/ed4807212c28c90777c1d7ef2bf8e47af5d08519) | Port observable typed-path changes in API validation and options, resolver candidates, module identities/specifiers, source maps, build-info spelling and watchers. Retain equivalent Rust string/Path storage and calculate canonical resolution keys at existing lookup sites; SDK and pinned binary fixtures refresh with the final pin. |
| [#64608](https://github.com/microsoft/TypeScript/commit/cb72b2e17b75557db7e3286d835544aef5d85f6c) | Localized Go diagnostics; tsrs documents English-only diagnostics. |
| [#64597](https://github.com/microsoft/TypeScript/commit/f9f8d01292562242b6e7c7142e46ed7e470926f9) | Port TokensAreOnSameLine to compare the next token start instead of its end. |
| [#64621](https://github.com/microsoft/TypeScript/commit/50d70a3f5f453a79a4323b263165da51f656a4e3) | Port CompareTypes ordering for inferred type parameters. |
| [#64620](https://github.com/microsoft/TypeScript/commit/6b3da828d7f59669ba3b26e9a466b7bde9a5f6d4) | Removal of legacy localized resources; no corresponding resources in tsrs. |
| [#64624](https://github.com/microsoft/TypeScript/commit/bcd7c422559da26b7e60a46a41cf5503a360d106) | Rust already stores its idle cleanup timer; no behavior change needed. |
| [#64632](https://github.com/microsoft/TypeScript/commit/ca197b7b8586900c3b75c0365fac31cfcaaacca7) | Port restarting imports when another arrival lowers the node_modules depth. |
| [#64646](https://github.com/microsoft/TypeScript/commit/a1ef42b9ea7032fa60df127d42b4c86fd2a110ee) | Port eager checking of constructor-defined properties for class declarations and deferred class expressions. |
| [#64584](https://github.com/microsoft/TypeScript/commit/fc636a691f1dbdbc9ba678911891a9e85ec283bf) | Port the async-disposable using diagnostic; the async client disposal Promise changes are included in the SDK refresh with the final pin. |
| [#64604](https://github.com/microsoft/TypeScript/commit/de61e696214322f72d967bd17c58c50e44b2920d) | Copy the merged DOM library generator update verbatim from the upstream commit. |
| [#64649](https://github.com/microsoft/TypeScript/commit/253bcd86231f2ceda21029eedf4c92c8b0d583f4) | Port checker emit-support layering, scoped resolvers and shared per-file/per-request contexts; retain Rust scratch regions and per-call nodebuilders rather than adding an unmeasured resolver cache. |
| [#64617](https://github.com/microsoft/TypeScript/commit/afd02f14863971d7a1b424f7c5a7e6a4eb2f92a7) | Port JSDocParameterTag type annotation lookup. |
| [#64636](https://github.com/microsoft/TypeScript/commit/1a78786ca751559170857893fe3e01eba7c69997) | Port import-type qualifier exclusion from local import reference marking. |
| [#64160](https://github.com/microsoft/TypeScript/commit/21b260b4e7e5123727f33d506fa686fecc258b1a) | Port top-level import filtering in document symbols. |
| [#64633](https://github.com/microsoft/TypeScript/commit/c3f14c2fe8971440d5a8bf1432d36bb26828c7c7) | Port object-literal parent tracking during decorator metadata emit. |
| [#64637](https://github.com/microsoft/TypeScript/commit/09d4966578aec90fdec11784559218392602a029) | Rust already returns None for missing config files; update the reference comment to reflect the upstream guard. |
| [#64638](https://github.com/microsoft/TypeScript/commit/d61a7d235906e78be9b111de0ba060e28e67ecfa) | Port the custom-resolution flag and disk-layout diagnostic guard. |
| [#64651](https://github.com/microsoft/TypeScript/commit/54f05ae22afd5f0ec5ec3ae0333b03efa1156fe6) | Port expression and object-literal element assertions for malformed assignments. |
| [#64675](https://github.com/microsoft/TypeScript/commit/91521cf2299d54a40d46fcf500130d273ddfc947) | Upstream prefilters Go test.run patterns; tsrs uses its own --filter/--list selector and has no Go testing-name normalization. |
| [#64647](https://github.com/microsoft/TypeScript/commit/4529d07ab5b7a2d2bbe0ef8b25edbed9f35b7b7f) | VS Code API client export; no tsrs extension. SDK source copies refresh with the final pin. |
| [#64640](https://github.com/microsoft/TypeScript/commit/bea2e849e90b5ff2889f9e120e1e0fae5e17659b) | Port deferred import recovery, namespace-binding grammar diagnostics, plain-JS diagnostics and default helper handling. |
| [#64670](https://github.com/microsoft/TypeScript/commit/0ba5d992db957fa444c3049e51243a056d3a9276) | Port raw private-identifier lookup and untransformed-kind guards for invalid class members. |
| [#64676](https://github.com/microsoft/TypeScript/commit/0bd74f77b276e659df9f63c3c63377cf11d1d86a) | Port normal CommonJS visitors for for-loop bodies. |
| [#64642](https://github.com/microsoft/TypeScript/commit/9055b588528600681abb8d95b3f1f39ef1707e4a) | Port inferred-project cleanup and program construction before workspace symbol queries. |
| [#64469](https://github.com/microsoft/TypeScript/commit/a501b11f64592a18287664ecc478a09659ad3134) | Port resolved-target helper; retain the equivalent ordered external-module scan instead of adding an unmeasured container index/cache under the repository performance rules. |
| [#64674](https://github.com/microsoft/TypeScript/commit/ec47d33c23e464a17cdf2475632cba629bee8763) | Port unconditional computed enum property checking and linked-reference handling. |
