# mem-round3: peak memory, third pass (representation and lazy creation)

Follow-up to notes/mem-layout.md, notes/mem-lazy.md and notes/mem-round2.md. Track A changes only the
representation (no checker semantics); track B prototypes Go-portable "created and never used" candidates on the
type side behind `TSRS_LAZY_*` switches.

Gates for every step: the suite (errors and `--baselines types,symbols`) byte-identical to the base commit, as whole
`target/test-results` trees, in the default mode, with `TSRS_LAZY_MEMBERS=0`, and with
`TS_TEST_PROGRAM_SINGLE_THREADED=false`; the `--extendedDiagnostics` counters unchanged in both modes; the private
monorepo 0 errors. Measurement: `/usr/bin/time -l` on the private-monorepo checkout, rounds interleaved, medians of 3,
"GiB" = peak memory footprint / 2^30; instructions retired are the CPU-work measure.

Status (2026-10-10): sizes are as of 49feed5 and many have shrunk since (notes/mem-layout3.md, mem-small.md,
mem-pointer-compression.md); B2 is still the default (`TSRS_LAZY_INFERENCE_MAPPERS`, crates/tsrs_core/src/lazymembers.rs).
The per-step tables, the occupancy counts and the before/after profiles were removed (git history has them).

## Track A (all landed)

| step | change | single GiB | 4 checkers GiB |
| --- | --- | --- | --- |
| A1 | resolved members of structured types in a record allocated on first write (4.9M of 7.6M structured types are never resolved) | -0.17 | -0.26 |
| A2 | relation cache keys: type-id pairs packed into 8-byte slots (below) | -0.08 | -0.18 |
| A3 | AST node header 32 -> 24 bytes (kind, data tag and parent in one word, 32-bit id) | -0.17 | -0.15 |
| A4 | symbol table entries 16 -> 8 bytes (symbol address, odd-key flag, length capped at 63, 12 hash bits) | -0.22 | -0.27 |
| A5 | no links slot for a read of an object literal member's `nameType` (`try_get`; tsrs-only, Go's paged store allocates the slot either way) | -0.03 | -0.02 |
| A6 | instantiated type aliases allocated only when a type is created with them (`AliasArg` / `PendingTypeAlias`; 0.55M of 2.34M records ended up on a type) | -0.06 | -0.08 |
| A7 | reference instantiation tables store only the references (below) | -0.06 | -0.06 |
| A8 | synthetic symbols keep containing and name types in the words of target and mapper | -0.07 | -0.08 |
| A9 | value-symbol links in three words (plain / synthetic / tail mode in the low bits) | -0.07 | -0.11 |
| A10 | pointer-keyed link stores: 12-byte slots, values in 1,024-value chunks (+0.3% instructions) | -0.03 | -0.01 |
| A11 | symbol table header 32 -> 24 bytes (`EntryVec`) | -0.04 | -0.06 |

### A2. Relation cache keys as type-id pairs

Go keys the relation caches by the 128-bit xxh3 hash of the `getRelationKey` bytes. The simple form (`'s'`, source
id, target id, intersection state) was 16.02M of 16.06M keys computed, so `get_relation_key` returns
`RelationKey::Pair` (source and target id below 2^28, state below 2^2, packed into 58 bits; an injective mapping)
without building and hashing the key bytes, and `RelationKey::Hashed` (the same 128-bit hash) for everything else.
`Relation` keeps the pairs in a `hashbrown::HashTable<u64>` whose slot is `key << 6 | result bits` (8 bytes + 1
control byte per slot instead of 17 + 1). The forms never alias: Go's `'s'` and `'g'` byte strings differ, and a
simple key is `Pair` exactly when it fits. Instructions -0.8% on 4 checkers.

### A7. Reference instantiation tables

For generic class, interface and tuple targets the instantiation map is keyed by `getTypeListKey(typeArguments)` and
every value is a reference whose `resolved_type_arguments` are exactly that list. `InterfaceType.instantiations` is a
`ReferenceInstantiations`: a `hashbrown::HashTable<P<Type>>` hashed over the type ids and compared by list equality,
one word per slot and no xxh3 per lookup. List equality is the injective version of Go's key (Go relies on the
128-bit hash having no collisions). The conditional and object-type caches map keys to arbitrary result types, so
they keep their keys.

## Track B: lazy creation on the type side

Same conventions as notes/mem-lazy.md: every candidate has a `TSRS_LAZY_*` switch in `tsrs_core::lazymembers` that
only acts when the master switch is on, so `TSRS_LAZY_MEMBERS=0` stays reference-identical. Attribution of the 9.63M
types and 17.3M mappers by creating call (private monorepo single) found that most are objects created on a cache miss
and kept by the type they built, so they are "used" by construction. Two had a measurable never-used share:

### B1 (rejected): defer the type of two-constituent union/intersection properties (`TSRS_LAZY_PROP_TYPES`, removed)

`createUnionOrIntersectionProperty` computes the property type eagerly unless there are more than two constituent
types (then it sets `CheckFlags::DeferredType` and keeps the constituents in `deferredSymbolLinks`). Of the eagerly
typed properties, 159K are ever read through `getTypeOfSymbol`; the unread ones made 339K new intersection types (541K
properties) and 25K new unions (67K properties). Deferring the two-constituent case as well gave 9,630,120 ->
9,275,217 types (-3.7%) but peak only 6.289 -> 6.275 GiB (the deferred-symbol links and constituent lists cost nearly
what the types did), and it changed results: the private monorepo reports TS2578 (unused `@ts-expect-error`) in one
test file, because code that reads `links.resolvedType` directly (e.g. `isSymbolUnaffectedByInstantiation`) and the
deferred-type paths behave differently. Not pursued.

### B2 (landed, default on): inference context mappers on first use (`TSRS_LAZY_INFERENCE_MAPPERS`)

Go's `newInferenceContextWorker` creates `context.mapper` and `context.nonFixingMapper` with every context. Now
`InferenceContext::mapper()` / `non_fixing_mapper()` create them on the first call (once, so the identity that
`findActiveMapper` and `compareTypeMappers` see is stable; nothing else about a mapper is observable). Of 1.43M
contexts on the private monorepo, 0.93M ever use the non-fixing mapper and 0.80M the fixing one: inference mappers 2.86M ->
1.73M. Upstream this saves the same 1.13M allocations per private-monorepo check (Go: allocation/GC work, not retained
memory). Peak -0.02 GiB single, -0.03 on 4 checkers.

```go
 func (c *Checker) newInferenceContextWorker(inferences []*InferenceInfo, signature *Signature, flags InferenceFlags, compareTypes TypeComparer) *InferenceContext {
 	n := &InferenceContext{inferences: inferences, signature: signature, flags: flags, compareTypes: compareTypes}
-	n.mapper = c.newInferenceTypeMapper(n, true /*fixing*/)
-	n.nonFixingMapper = c.newInferenceTypeMapper(n, false /*fixing*/)
 	return n
 }
+
+func (c *Checker) getFixingMapper(n *InferenceContext) *TypeMapper {
+	if n.mapper == nil {
+		n.mapper = c.newInferenceTypeMapper(n, true /*fixing*/)
+	}
+	return n.mapper
+}
```
(and `getNonFixingMapper` likewise; the ~15 reads of `n.mapper` / `n.nonFixingMapper` go through them.)

## Tried and not landed

- **Deferred two-constituent property types (B1)**: see above (-3.7% types, -0.014 GiB, changes results).
- **Lazy member tables in the arena instead of `Rc`** (128-byte packed table, no reference counts): arena +155 MB,
  heap -141 MB, peak +0.01 GiB single / +0.03 GiB 4 checkers (the `Rc` blocks fit a 176-byte size class exactly,
  and the 12% of tables dropped when resolved in full are reused heap). Instructions -0.3%. The same packing
  inside the `Rc` (176 -> 144-byte blocks): -0.01 / -0.02 GiB, not worth the churn.
- **Symbol tables searched linearly up to 32 entries** (re-measured with 8-byte entries): -0.02 GiB, +0.3-0.6%
  instructions; stays at 16.
- **A first version of A10** (lookup then insert, bounds-checked chunk access): +0.6% instructions for -0.03 GiB;
  the landed version uses the hash table's entry API (+0.3%).
- **Interning 1-2 element type-argument lists**: not built; mem-round2 measured 5.35M distinct of 10M lists, and
  for short lists the dedup table's entry (16+ bytes) costs more than the 8-16 bytes a duplicate saves.
- **Identifier text from the source**, **Symbol 56 -> 48 bytes**: not done here; notes/mem-small.md did both later.

## Result

Interleaved, 3 rounds each (base = dc59d8e; final = 49feed5). The final binary also contains the concurrent checker
CPU pass (notes/cpu-checker.md), which accounts for the instruction drop; the steps of this pass were each within
-0.8% (A2) .. +0.5% (A3) instructions of their predecessor.

| run | base peak GiB | final peak GiB | base instructions | final instructions |
| --- | --- | --- | --- | --- |
| default, single | 7.017 | 6.022 (-14.2%) | 323 G | 297 G |
| default, 4 checkers | 9.384 | 8.069 (-14.0%) | 434-438 G | 400-402 G |
| opt-out, single | 9.47 | 8.01 (-15.4%) | 345 G | 318 G |
| opt-out, 4 checkers (go assignment) | 14.53 | 12.35 (-15.0%) | 520 G | 481 G |
