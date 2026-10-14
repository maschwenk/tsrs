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
