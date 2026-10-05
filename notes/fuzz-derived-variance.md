# fuzz-derived-variance: attacking TSRS_DERIVED_VARIANCE

Work in progress. Findings so far (each confirmed against tsgo built from the pinned commit; the switch off and
tsgo agree, `on` loses the error, `shadow` exits 7):

| case | cause |
| --- | --- |
| `testdata/regressions/derived-variance-any-keyof` | `any` argument: `keyof any` and a homomorphic mapped type over `any` evaluate eagerly; the markers kept them deferred |
| `testdata/regressions/derived-variance-keyof-optional` | `{}` and `{ a?: string }` are mutually assignable, `keyof` tells them apart (no `any`, no conditional) |
| `testdata/regressions/derived-variance-mutual-conditional` | a conditional's check type measures bivariant; mutually assignable arguments pick different branches |
| `testdata/regressions/derived-variance-this-conditional` | `this` in a conditional's check type measures bivariant; the derived type picks the other branch (identical arguments) |
| `testdata/regressions/derived-variance-any-template` | `any` in a template literal type gives `` `a${any}` `` |
