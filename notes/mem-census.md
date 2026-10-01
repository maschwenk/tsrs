# mem-census: how much of tsrs's memory is garbage at the end of a check

Go frees inference contexts, temporary mappers, key lists, scratch tables and discarded parse results with its GC;
the tsrs leak arena (`tsrs_core::P<T>`) keeps every arena object, and the Rust heap keeps every block whose owner
is an arena object (destructors never run). This pass measures that share: memory still held at exit that nothing
reachable points to, by arena type, by arena call site, by allocating function (sampled stacks) and by heap
allocating function. No recycling is implemented here; the ranked list at the end is the input for the next pass.

## Method

Build the alloc-profile binary (`CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features
alloc-profile`) and run with `TSRS_CENSUS=1` (code: `crates/tsrs_core/src/alloc_profile/census.rs`, hook in
`crates/tsrs_cli/src/census.rs`; compiled out of normal builds).

- **Recording.** `record` (every `P::new` / `alloc*`) now also gets the block address and, with the census on,
  appends (address, size, site + type) to a per-thread list; every 16th arena allocation
  (`TSRS_CENSUS_ARENA_SAMPLE`) also records its raw stack, because `#[track_caller]` sites are one level deep and
  stop at wrappers (`new_node`, `Type::alloc`, `Symbol::new`, link stores). The counting allocator keeps every live
  heap block (address, size, interned raw stack, 16 frames) in a sharded map; arena chunks are not blocks. Census
  bookkeeping allocates with tracking suspended.
- **Clearing freed memory.** While recording, freed heap memory is zeroed before it is returned to mimalloc
  (realloc moves by hand to do the same). Without it, stale words in reused memory (`Vec` slack, empty hash
  buckets) kept arbitrary blocks alive. The first version did not clear: inference contexts read 77% unreachable
  by exact site but 54% in the sampled table, because the freed buffers of the census's own sample lists (which
  hold the sampled addresses) were reused by tracked blocks. With clearing the two agree (84.2% / 84.2%).
- **Roots.** At the end of `perform_compilation` (after diagnostics are printed; the `Program`, config and
  diagnostics are still live): the explicit roots `Program`, `ParsedCommandLine` and the diagnostics vector, the
  current thread's stack (from the census frame up), and the main image's `__DATA_CONST` / `__DATA` segments (all
  statics, including `OnceLock` / `LazyLock` contents). Thread-locals are not scanned: none holds program data
  (`ARENA` points at the `Bump`, whose chunks are not blocks; the profiler's own tables hold only static data).
  Other threads' stacks are not scanned (with `--checkers 1` the checker thread has exited; its `Checker` lives
  in the pool owned by `Program`; idle rayon workers keep a few deque blocks, which show up as ~0.1 MB of false
  garbage under `rayon_core`).
- **Mark.** Blocks are sorted by address with a two-level page index (1 GiB regions, 4 KiB pages: first block per
  page). Every 4-byte-aligned word of a reachable block is decoded two ways: the low 48 bits (plain pointers,
  low-bit tags of mappers and value-symbol links, `PackedStr` and mapper slice lengths in the top 16 bits) and
  the low 45 bits times 8 (`NodeHeaderWord` parents, `SymbolMapEntry` symbols). 4-byte steps cover `SliceCell` /
  `OwnedSliceCell` / pointer-keyed link slots. Interior pointers count (hash tables point at their control
  bytes, slices into link chunks). Arena blocks of pointer-free types (`str`, `PackedStr`, `[u8]`, `[i32]`, ...)
  are not scanned.
- **Conservative.** Any word that looks like a pointer keeps its target alive, so every unreachable number is a
  lower bound. Run-to-run variation is about +-3 MB, except the dropped parse results (below), which vary more.
- **Check** (`TSRS_CENSUS_VERIFY=1`): walks every source file's AST (`for_each_child`), every node's symbol, local
  symbol and locals, every member / export table and every declaration, and asks whether the census found each
  reachable. On the private monorepo: 25,279,722 nodes and 3,906,182 binder symbols checked, 0 unreachable, 0 not
  recorded, in both modes. Same on a 5-line file and on `compiler/inferFromGenericFunctionReturnTypes3.ts`.
  Two unreachable rows were also checked by hand. `ExpressionWithTypeArguments` from
  `parse_type_heritage_clause_element` (100% unreachable on the small programs): interface `extends` elements
  are converted to `TypeReferenceNode`s and the parsed node is dropped. `Identifier`s from
  `parse_type_or_type_predicate` (99%): `parseTypePredicatePrefix` runs under `tryParse` and is rolled back.
- **Cost.** The private monorepo with one checker: 73 s instead of 22 s, peak 13.4 GB instead of 6.8 GB. The mark
  takes 18 s (default mode) / 29 s (opt-out). `TSRS_CENSUS_TOP=N` sets the table length.
  `TSRS_CENSUS_TSV=<path>` writes every row of every table for offline grouping. With the census on, the
  alloc-profile's "heap" totals include census bookkeeping; the census's own heap numbers do not.

## Totals (the private monorepo, `--checkers 1`, main at d8d636e + this change)

Normal binary: 22.1 s, peak footprint 6,802,906,576 B (6.34 GiB), output byte-identical to the base binary.

| mode | arena allocated | arena unreachable | heap live at exit | heap unreachable | total unreachable |
| --- | --- | --- | --- | --- | --- |
| default (lazy members) | 4,572 MB / 115.4M blocks | 661 MB (14.5%) / 25.4M | 1,592 MB / 8.10M | 82 MB (5.1%) / 2.15M | 743 MB |
| `TSRS_LAZY_MEMBERS=0` | 6,088 MB / 140.3M blocks | 754 MB (12.4%) / 31.4M | 2,100 MB / 8.97M | 83 MB (3.9%) / 2.15M | 837 MB |

(MB = 2^20 bytes. Arena chunks are 6,406 / 10,518 MB: bumpalo doubles chunk sizes and the untouched tail of the
last chunk is not resident.) So about 0.7 GiB of the 6.3 GiB peak (11%) is garbage that Go would have collected.
That is the ceiling for recycling and scoping; the rest of the peak is live.

## Garbage by cause (default mode; from the sampled stacks, scaled x16)

Rows grouped by type and the first allocating frames (rules applied to the `TSRS_CENSUS_TSV` rows: inference types and
inference mappers by the function that made the context; everything under `Parser::`; binder flow nodes; object
literal and spread types from expression checking; the remaining mappers and type lists). Sampled totals
match the exact totals (661 MB). The sampled estimate is noisy for few large blocks, such as source texts.

| unreachable MB | of allocated MB | cause |
| --- | --- | --- |
| 127.5 | 225.4 | mappers outside inference: `appendTypeMapping` in `getTypeOfMappedSymbol` / `getIndexTypeForMappedType` / `resolveMappedTypeMembers` (~51 MB, ~100% garbage), conditional-type instantiation mappers (`getConditionalTypeInstantiation` 20 MB, `getConditionalType` composite 15 MB, `getTailRecursionRoot` 6 MB), `prependTypeMapping` in `instantiateMappedType`'s `mapTypeWithAlias` callback (16 MB) |
| 205.0 | 243.4 | inference: contexts, infos, `[P<InferenceInfo>]`, candidate lists (`LazyVec` `RefCell<Vec>`), inference mappers. By creator: `getConditionalType` (`infer`) 78.9, `chooseOverload` 73.9 (94% of its contexts are garbage), `inferTypeArguments` and context clones 25.9, candidate lists and `getInferredType` mappers 26.3. Plus 5.5 MB heap (candidate `Vec` buffers) |
| 83.9 + 37.9 + ~45 heap | 155 (types, members, tables) | expression re-checks: object literal types, their `StructuredMembers`, property arrays, member `SymbolTable`s (`checkExpressionWithContextualType` from `isSignatureApplicable` / `inferTypeArguments`, 99.5% garbage there; `checkExpressionForMutableLocation`), the property `Symbol`s (37.9 MB, 68% of the symbols `checkExpressionWorker` makes), spread types; heap: the symbol table entry buffers of the dead tables (`EntryVec::set_capacity` 44 MB) |
| 76.9 | 1,283 | parser: speculative parses rolled back by `tryParse` / `lookAhead` (`parseTypePredicatePrefix`, parameter lists, object binding patterns, entity names), the diagnostics those speculations produce (`parse_object_binding_pattern` 5.4 MB, 100%), heritage `ExpressionWithTypeArguments`; also the ASTs of 450-950 files (varies by run) whose parsed `SourceFile` is not kept (their texts: 8.4 MB of heap `read_file` buffers; cause not traced) |
| 55.4 | 215.4 | type lists: `getConditionalTypeInstantiation` type arguments (24.8 MB, 79%), `getObjectTypeInstantiation` (8.0, 30%), `getTailRecursionRoot` (7.3, 99%), union constituent lists of unions that die (6.6) |
| 24.9 | 1,044 | types built during instantiation that die: unions from `instantiateType` (17.3 MB, 74%), instantiated signatures (3.5) |
| 22.9 | 79.5 | binder flow nodes and lists: `bindIfStatement` / condition labels, `createFlowCondition`, `addAntecedent` lists for labels that end up unused |
| 15.0 + 7.8 + 1.0 | | other: `ExportCollision` (3.3 MB, 100%), declaration-array growth in `Symbol::append_declarations` (Go `append` garbage, 2.2), `WideningContext`, synthetic call arguments, `KnownDirectoryLink` + 7.3 MB heap path strings in `KnownSymlinks::process_resolution` (96%); node builder for diagnostic text (7.8); checker diagnostics (1.0) |

Opt-out mode (`TSRS_LAZY_MEMBERS=0`) adds 68 MB of composite mappers from `instantiateTypeWithAlias`, which lazy
member tables avoid. It also adds 35 MB of eagerly created inference mappers (B2 in notes/mem-round3.md is off
there). The rest matches: mappers 196.8, inference 222.8, expression re-checks 84.7 + 41.4, parser 76.8, type
lists 61.3, binder 22.9 MB.

## Tables (default mode, top 30 by unreachable bytes)

Arena by type:

| unreach MB | unreach count | alloc MB | alloc count | un% | type |
| --- | --- | --- | --- | --- | --- |
| 149.4 | 9,791,328 | 253.5 | 16,614,881 | 59 | `TypeMapper` |
| 76.3 | 1,249,523 | 90.4 | 1,481,378 | 84 | `InferenceContext` |
| 71.3 | 1,558,411 | 83.9 | 1,833,792 | 85 | `InferenceInfo` |
| 54.2 | 3,957,977 | 215.9 | 12,209,448 | 25 | `[P<Type>]` |
| 40.8 | 763,526 | 702.9 | 13,161,548 | 6 | `Symbol` |
| 27.2 | 1,188,862 | 82.7 | 3,614,543 | 33 | `SymbolTable` |
| 25.2 | 659,456 | 309.6 | 8,115,017 | 8 | `NodeAlloc<Identifier>` |
| 24.7 | 463,081 | 165.9 | 3,106,131 | 15 | `TypeAlloc<ObjectType>` |
| 21.9 | 716,533 | 23.8 | 778,788 | 92 | `RefCell<Vec<P<Type>>>` |
| 21.2 | 462,926 | 124.4 | 2,716,787 | 17 | `StructuredMembers` |
| 17.6 | 153,806 | 120.5 | 1,053,244 | 15 | `TypeAlloc<UnionType>` |
| 16.3 | 534,621 | 58.3 | 1,911,282 | 28 | `FlowNode` |
| 13.5 | 498,922 | 97.5 | 2,794,093 | 14 | `[P<Symbol>]` |
| 11.9 | 1,249,511 | 14.0 | 1,481,378 | 85 | `[P<InferenceInfo>]` |
| 9.6 | 66,083 | 12.5 | 86,389 | 76 | `Diagnostic` |
| 6.7 | 87,409 | 36.0 | 472,019 | 19 | `NodeAlloc<ParameterDeclaration>` |
| 6.6 | 430,912 | 9.5 | 625,134 | 69 | `FlowList` |
| 5.9 | 248,093 | 106.0 | 9,049,348 | 6 | `[P<Node>]` |
| 5.9 | 76,825 | 13.2 | 173,572 | 44 | `NodeAlloc<BindingElement>` |
| 4.6 | 120,249 | 38.2 | 1,001,936 | 12 | `NodeAlloc<TypeReferenceNode>` |
| 4.3 | 189,103 | 94.0 | 4,106,284 | 5 | `NodeList` |
| 3.9 | 46,030 | 135.4 | 1,613,053 | 3 | `Signature` |
| 3.7 | 9,157 | 3.7 | 9,161 | 100 | `NodeBuilderContext` |
| 3.3 | 53,314 | 3.3 | 53,317 | 100 | `ExportCollision` |
| 3.0 | 132,451 | 42.5 | 1,858,875 | 7 | `Node` |
| 2.5 | 40,691 | 3.6 | 58,232 | 70 | `InferenceContextRare` |
| 1.8 | 26,709 | 33.8 | 491,791 | 5 | `NodeAlloc<PropertySignatureDeclaration>` |
| 1.7 | 24,384 | 1.7 | 24,998 | 98 | `WideningContext` |
| 1.5 | 58,785 | 61.8 | 3,036,584 | 2 | `str` |
| 1.3 | 10,169 | 11.1 | 85,246 | 12 | `NodeAlloc<MethodDeclaration>` |

Arena by call site (one level, `#[track_caller]`; `ast.rs:109` is `new_node`, `types.rs:1189` `Type::alloc`,
`symbol.rs:48` `Symbol::new`, `checker_11.rs:1424` the type-argument list in `getConditionalTypeInstantiation`,
`checker_11.rs:1259` the one in `getObjectTypeInstantiation`, `checker_12.rs:1183` in `getTailRecursionRoot`,
`mapper.rs:299/335/306/261/341` simple / merged / array / inference / composite mappers, `checker.rs:446`
`LazyVec::push`):

| unreach MB | unreach count | alloc MB | alloc count | un% | site |
| --- | --- | --- | --- | --- | --- |
| 76.3 | 1,249,523 | 90.4 | 1,481,378 | 84 | `tsrs_checker/src/inference.rs:1451  InferenceContext` |
| 64.1 | 1,400,609 | 74.8 | 1,634,852 | 86 | `tsrs_checker/src/inference.rs:1872  InferenceInfo` |
| 46.5 | 3,049,370 | 79.3 | 5,197,810 | 59 | `tsrs_checker/src/mapper.rs:299  TypeMapper` |
| 40.8 | 763,525 | 702.9 | 13,161,547 | 6 | `tsrs_ast/src/symbol.rs:48  Symbol` |
| 38.1 | 2,499,157 | 43.6 | 2,855,326 | 88 | `tsrs_checker/src/mapper.rs:335  TypeMapper` |
| 27.1 | 1,182,057 | 44.9 | 1,960,718 | 60 | `tsrs_ast/src/symbol.rs:549  SymbolTable` |
| 25.2 | 659,456 | 309.6 | 8,115,017 | 8 | `tsrs_ast/src/ast.rs:109  NodeAlloc<Identifier>` |
| 24.7 | 463,081 | 165.9 | 3,106,131 | 15 | `tsrs_checker/src/types.rs:1189  TypeAlloc<ObjectType>` |
| 24.4 | 1,614,974 | 31.0 | 1,956,998 | 79 | `tsrs_checker/src/checker_11.rs:1424  [P<Type>]` |
| 24.2 | 1,583,722 | 64.2 | 4,206,923 | 38 | `tsrs_checker/src/mapper.rs:306  TypeMapper` |
| 21.2 | 462,926 | 124.4 | 2,716,787 | 17 | `tsrs_checker/src/types.rs:1757  StructuredMembers` |
| 20.8 | 1,362,510 | 27.2 | 1,781,435 | 76 | `tsrs_checker/src/mapper.rs:261  TypeMapper` |
| 19.6 | 640,886 | 20.8 | 683,131 | 94 | `tsrs_checker/src/checker.rs:446  RefCell<Vec<P<Type>>>` |
| 17.6 | 153,806 | 120.5 | 1,053,244 | 15 | `tsrs_checker/src/types.rs:1189  TypeAlloc<UnionType>` |
| 17.5 | 1,148,560 | 35.2 | 2,309,698 | 50 | `tsrs_checker/src/mapper.rs:341  TypeMapper` |
| 16.0 | 522,938 | 57.9 | 1,898,158 | 28 | `tsrs_binder/src/binder.rs:116  FlowNode` |
| 12.6 | 451,124 | 74.2 | 1,430,795 | 17 | `tsrs_checker/src/checker_12.rs:2025  [P<Symbol>]` |
| 11.9 | 1,249,511 | 14.0 | 1,481,378 | 85 | `tsrs_checker/src/inference.rs:1451  [P<InferenceInfo>]` |
| 9.6 | 66,079 | 12.5 | 86,166 | 77 | `tsrs_ast/src/diagnostic.rs:224  Diagnostic` |
| 8.0 | 1,036,117 | 27.7 | 2,123,956 | 29 | `tsrs_checker/src/checker_11.rs:1259  [P<Type>]` |
| 7.2 | 157,802 | 9.1 | 198,940 | 79 | `tsrs_checker/src/inference.rs:1883  InferenceInfo` |
| 7.2 | 367,201 | 7.2 | 369,655 | 99 | `tsrs_checker/src/checker_12.rs:1183  [P<Type>]` |
| 6.8 | 153,798 | 40.0 | 1,053,244 | 17 | `tsrs_checker/src/checker_12.rs:2070  [P<Type>]` |
| 6.7 | 87,409 | 36.0 | 472,019 | 19 | `tsrs_ast/src/ast.rs:109  NodeAlloc<ParameterDeclaration>` |
| 6.6 | 430,912 | 9.5 | 625,134 | 69 | `tsrs_binder/src/binder.rs:615  FlowList` |
| 5.9 | 76,825 | 13.2 | 173,572 | 44 | `tsrs_ast/src/ast.rs:109  NodeAlloc<BindingElement>` |
| 4.6 | 120,249 | 38.2 | 1,001,936 | 12 | `tsrs_ast/src/ast.rs:109  NodeAlloc<TypeReferenceNode>` |
| 4.3 | 189,103 | 94.0 | 4,106,284 | 5 | `tsrs_ast/src/ast.rs:197  NodeList` |
| 3.9 | 46,029 | 135.4 | 1,613,052 | 3 | `tsrs_checker/src/checker_12.rs:2150  Signature` |
| 3.7 | 9,157 | 3.7 | 9,161 | 100 | `tsrs_checker/src/nodebuilder.rs:31  NodeBuilderContext` |

Arena by type and allocating function <- callers (sampled 1/16, scaled):

| unreach MB | unreach count | alloc MB | alloc count | un% | site |
| --- | --- | --- | --- | --- | --- |
| 34.2 | 559,696 | 36.2 | 593,552 | 94 | `InferenceContext  Checker::new_inference_context_worker  <-  Checker::new_inference_context  <-  Checker::choose_overload` |
| 31.8 | 595,952 | 46.9 | 879,008 | 68 | `Symbol  Symbol::new  <-  Checker::check_expression_worker  <-  Checker::check_expression_with_contextual_type` |
| 30.5 | 499,552 | 41.4 | 677,584 | 74 | `InferenceContext  Checker::new_inference_context_worker  <-  Checker::new_inference_context  <-  Checker::get_conditional_type` |
| 29.0 | 633,776 | 37.4 | 816,032 | 78 | `InferenceInfo  new_inference_info  <-  Checker::new_inference_context  <-  Checker::get_conditional_type` |
| 28.5 | 622,752 | 31.0 | 677,024 | 92 | `InferenceInfo  new_inference_info  <-  Checker::new_inference_context  <-  Checker::choose_overload` |
| 18.3 | 400,480 | 31.5 | 688,576 | 58 | `StructuredMembers  Checker::set_structured_type_members  <-  create_object_literal_type  <-  Checker::check_expression_worker` |
| 17.5 | 328,528 | 18.7 | 350,640 | 94 | `TypeAlloc<ObjectType>  create_object_literal_type  <-  Checker::check_expression_worker  <-  Checker::check_expression_with_contextual_type` |
| 16.2 | 1,060,000 | 20.3 | 1,329,712 | 80 | `TypeMapper  prepend_type_mapping  <-  Checker::map_type_with_alias::_{{closure}}  <-  Checker::map_type_ex_worker` |
| 11.6 | 506,576 | 11.6 | 508,176 | 100 | `SymbolTable  Checker::check_expression_worker  <-  Checker::check_expression_with_contextual_type  <-  Checker::is_signature_applicable` |
| 11.4 | 375,024 | 11.6 | 381,360 | 98 | `RefCell<Vec<P<Type>>>  LazyVec<T>::push  <-  Checker::infer_from_types  <-  Checker::infer_types` |
| 11.1 | 96,848 | 15.0 | 130,752 | 74 | `TypeAlloc<UnionType>  Checker::new_union_type  <-  Checker::get_union_type_worker  <-  Checker::instantiate_type_with_alias` |
| 10.5 | 690,528 | 10.5 | 690,736 | 100 | `TypeMapper  append_type_mapping  <-  Checker::get_type_of_symbol  <-  Checker::get_property_type_for_index_type` |
| 9.7 | 633,072 | 9.7 | 633,104 | 100 | `TypeMapper  append_type_mapping  <-  Checker::get_index_type_for_mapped_type  <-  Checker::get_index_type_ex` |
| 9.2 | 600,592 | 9.2 | 600,640 | 100 | `TypeMapper  append_type_mapping  <-  Checker::resolve_mapped_type_members  <-  Checker::resolve_structured_type_members_worker` |
| 9.1 | 596,656 | 10.4 | 680,112 | 88 | `TypeMapper  new_inference_type_mapper  <-  Checker::get_conditional_type  <-  Checker::map_type_ex_worker` |
| 9.1 | 595,536 | 10.4 | 682,496 | 87 | `TypeMapper  new_composite_type_mapper  <-  Checker::get_conditional_type  <-  Checker::map_type_ex_worker` |
| 8.7 | 568,800 | 8.7 | 568,880 | 100 | `TypeMapper  append_type_mapping  <-  Checker::get_index_type_for_mapped_type  <-  Checker::instantiate_type_with_alias` |
| 7.6 | 389,392 | 12.6 | 665,680 | 60 | `[P<Symbol>]  Checker::set_structured_type_members  <-  create_object_literal_type  <-  Checker::check_expression_worker` |
| 7.2 | 521,664 | 9.0 | 606,768 | 80 | `[P<Type>]  Checker::get_conditional_type_instantiation_ex  <-  Checker::instantiate_type_with_alias  <-  Checker::instantiate_list` |
| 7.1 | 464,864 | 9.0 | 592,544 | 78 | `TypeMapper  Checker::get_conditional_type_instantiation_ex  <-  Checker::instantiate_type_with_alias  <-  Checker::instantiate_list` |
| 6.2 | 406,128 | 10.2 | 665,376 | 61 | `TypeMapper  new_composite_type_mapper  <-  Checker::get_conditional_type  <-  Checker::get_conditional_type_instantiation_ex` |
| 5.8 | 753,664 | 19.1 | 1,564,944 | 30 | `[P<Type>]  Checker::get_object_type_instantiation  <-  Checker::instantiate_type_with_alias  <-  Checker::get_type_of_symbol` |
| 5.6 | 367,472 | 9.5 | 625,344 | 59 | `TypeMapper  new_inference_type_mapper  <-  Checker::get_conditional_type  <-  Checker::get_conditional_type_instantiation_ex` |
| 5.5 | 90,000 | 5.5 | 90,048 | 100 | `InferenceContext  Checker::new_inference_context_worker  <-  Checker::new_inference_context  <-  Checker::infer_type_arguments` |
| 5.4 | 37,008 | 5.4 | 37,040 | 100 | `Diagnostic  new_diagnostic  <-  Parser::parse_expected_with_diagnostic  <-  Parser::parse_object_binding_pattern` |
| 5.3 | 116,624 | 5.3 | 116,736 | 100 | `InferenceInfo  new_inference_info  <-  Checker::new_inference_context  <-  Checker::infer_type_arguments` |
| 5.3 | 96,016 | 6.3 | 125,248 | 85 | `[P<Type>]  Checker::new_union_type  <-  Checker::get_union_type_worker  <-  Checker::instantiate_type_with_alias` |
| 5.3 | 268,608 | 5.4 | 270,976 | 99 | `[P<Type>]  Checker::get_tail_recursion_root  <-  Checker::get_conditional_type  <-  Checker::get_conditional_type_instantiation_ex` |
| 5.3 | 115,120 | 6.6 | 144,336 | 80 | `InferenceInfo  clone_inference_info  <-  Checker::clone_inference_context  <-  Checker::infer_type_arguments` |
| 5.1 | 286,160 | 5.2 | 287,056 | 100 | `[P<Type>]  Checker::get_conditional_type_instantiation_ex  <-  Checker::instantiate_type_with_alias  <-  Checker::get_type_of_symbol` |

Heap blocks live at exit, by allocating function <- caller:

| unreach MB | unreach count | alloc MB | alloc count | un% | site |
| --- | --- | --- | --- | --- | --- |
| 44.4 | 1,117,267 | 186.0 | 3,495,129 | 24 | `EntryVec::set_capacity  <-  SymbolMap::insert` |
| 8.4 | 855 | 216.2 | 39,693 | 4 | `_<os::DirFS as internal::IoFS>::read_file  <-  internal::Common::read_file` |
| 7.3 | 44,472 | 7.6 | 46,239 | 96 | `path::ensure_trailing_directory_separator  <-  knownsymlinks::KnownSymlinks::process_resolution` |
| 5.5 | 636,885 | 5.9 | 683,131 | 94 | `LazyVec<T>::push  <-  Checker::infer_from_types` |
| 3.8 | 15,809 | 6.4 | 28,184 | 60 | `SymbolMap::insert  <-  Checker::get_spread_type` |
| 1.7 | 36,565 | 1.7 | 36,644 | 100 | `NodeBuilderContext::new  <-  NodeBuilder::enter_context` |
| 1.6 | 83,790 | 3.9 | 115,123 | 41 | `stringify_args  <-  new_diagnostic` |
| 1.4 | 7,806 | 1.4 | 8,085 | 97 | `GoMap<K,V>::set  <-  Checker::get_widened_type_with_context` |
| 0.9 | 8,597 | 0.9 | 8,649 | 99 | `NodeBuilderImpl::create_anonymous_type_node_ex  <-  NodeBuilderImpl::type_to_type_node` |
| 0.9 | 53,247 | 0.9 | 53,317 | 100 | `get_text_of_node_from_source_text  <-  Checker::extend_export_symbols` |
| 0.9 | 3,697 | 40.8 | 208,230 | 2 | `fileLoader::resolve_imports_and_module_augmentations  <-  filesParser::parse` |
| 0.9 | 3,313 | 0.9 | 3,343 | 96 | `stringify_args  <-  Relater::report_error` |
| 0.7 | 5,241 | 1.0 | 7,006 | 73 | `SymbolMap::insert  <-  Checker::check_expression_worker` |
| 0.6 | 11,151 | 3.0 | 62,797 | 21 | `packagejson::parse  <-  ResolutionState::get_package_json_info` |
| 0.5 | 1,023 | 0.9 | 1,586 | 60 | `SymbolMap::insert  <-  Checker::extend_export_symbols` |
| 0.4 | 43,073 | 0.5 | 55,073 | 82 | `clone_inference_info  <-  <deduplicated_symbol>` |
| 0.4 | 1,842 | 14.8 | 79,546 | 3 | `parse_source_file_static  <-  _<compilerHost as CompilerHost>::get_source_file` |
| 0.3 | 6,185 | 1.4 | 39,886 | 19 | `_<EntryVec as core::clone::Clone>::clone  <-  SymbolTable::clone_table` |
| 0.3 | 32,163 | 0.3 | 40,584 | 80 | `clone_inference_info  <-  Checker::clone_inference_context` |
| 0.2 | 6,502 | 0.2 | 6,515 | 100 | `SymbolTrackerImpl::track_symbol  <-  NodeBuilderImpl::symbol_to_type_node` |

Opt-out mode, arena by type (top 15):

| unreach MB | unreach count | alloc MB | alloc count | un% | type |
| --- | --- | --- | --- | --- | --- |
| 237.3 | 15,552,442 | 340.1 | 22,289,888 | 70 | `TypeMapper` |
| 76.3 | 1,250,259 | 90.5 | 1,482,022 | 84 | `InferenceContext` |
| 71.4 | 1,558,823 | 84.0 | 1,834,467 | 85 | `InferenceInfo` |
| 59.2 | 4,211,027 | 233.6 | 12,701,057 | 25 | `[P<Type>]` |
| 41.1 | 769,880 | 1430.4 | 26,782,935 | 3 | `Symbol` |
| 27.3 | 1,191,491 | 110.8 | 4,841,912 | 25 | `SymbolTable` |
| 25.2 | 660,012 | 309.6 | 8,115,017 | 8 | `NodeAlloc<Identifier>` |
| 24.9 | 465,783 | 166.0 | 3,107,807 | 15 | `TypeAlloc<ObjectType>` |
| 21.9 | 717,160 | 23.8 | 779,379 | 92 | `RefCell<Vec<P<Type>>>` |
| 21.3 | 465,592 | 168.1 | 3,672,794 | 13 | `StructuredMembers` |
| 17.6 | 154,017 | 120.6 | 1,053,499 | 15 | `TypeAlloc<UnionType>` |
| 16.3 | 534,628 | 58.3 | 1,911,282 | 28 | `FlowNode` |
| 13.5 | 501,627 | 347.9 | 3,747,491 | 4 | `[P<Symbol>]` |
| 11.9 | 1,250,232 | 14.0 | 1,482,022 | 85 | `[P<InferenceInfo>]` |
| 9.6 | 66,067 | 12.5 | 86,389 | 76 | `Diagnostic` |

## Opportunities, ranked by expected savings (default mode, single checker)

The savings are upper bounds: the unreachable bytes of the class. All of these objects are also garbage in Go,
so reclaiming them changes no checker result. What has to be proven differs per class.

1. **Inference contexts and everything hanging off them: up to ~210 MB** (contexts 76, infos 71 including 7 from clones,
   `[P<InferenceInfo>]` 12, candidate `RefCell<Vec>` 22 + 5.5 heap, `InferenceContextRare` 2.5, inference mappers
   21). Created in `newInferenceContextWorker` / `newInferenceInfo` (inference.rs:1451, 1872) for
   `chooseOverload` candidates (94% garbage), `getConditionalType` `infer` matching (74%) and
   `inferTypeArguments` clones. Their last use is the end of `inferTypes` / `getInferredTypes` for that
   candidate or conditional type. A context escapes only through its fixing / non-fixing mapper, once created and
   stored (16% of the contexts stay reachable, and 0.42M of the 1.78M inference mappers do: about two per
   surviving context), or through
   a clone's or outer context's reference. Scoping requires an escape bit on mappers (item 2). Then
   `chooseOverload` (after each candidate) and `getConditionalType` (after the `infer` inference) can return a
   context whose mappers never escaped, with its infos, candidate vectors and info slice, to a per-checker free
   list. The B2 lazy mappers (notes/mem-round3.md) already make the "no mapper created" case provable without
   the bit (at most ~0.55M of the 1.48M contexts).
2. **Scratch mappers and their type lists: ~130 MB of mappers + ~40-55 MB of lists.** Mappers built only to feed
   one `instantiateType` call whose result is cached by type ids or does not keep the mapper.
   `appendTypeMapping` in `getTypeOfMappedSymbol`, `getIndexTypeForMappedType` and
   `resolveMappedTypeMembers` (`nameType` instantiation) is ~100% garbage. So are the miss paths of
   `getConditionalTypeInstantiation` (mapper + `alloc_slice(type_arguments)`, 79%), `getTailRecursionRoot` (99%),
   `getConditionalType`'s composite / inference mappers (61-88%), and `instantiateMappedType`'s
   `prependTypeMapping` (80%). Mapper identity is observable (`findActiveMapper`, `compareTypeMappers`, the
   active-mapper caches), so interning stays out (notes/mem-round2.md). Recycling needs a store barrier instead:
   every place that keeps a mapper beyond the call sets an "escaped" bit on it, and transitively on its children.
   Those places are `ObjectType.mapper` of instantiated anonymous / mapped / reverse types, deferred references,
   `ConditionalType.mapper` / `combinedMapper`, `Signature.mapper`, instantiated symbols' links, inference
   contexts' return mappers, and any cache keyed by a mapper (roughly 60 store sites in `tsrs_checker`). A
   scratch site frees the mapper and the lists it owns when the bit is clear after the call. The per-size free
   lists live in the checker (all of this is checker-owned memory).
3. **Expression re-checks under a temporary contextual type: ~120 MB arena + ~45 MB heap.** Object literal types,
   members, property symbols and member tables that `checkExpressionWithContextualType` (from
   `isSignatureApplicable`, `inferTypeArguments`) and `checkExpressionForMutableLocation` create per candidate
   and drop (types 94%, member tables 99.5%, property symbols 68%). This is the largest class that is not
   inference, and the hardest: the fresh types reach union / intersection / widening / relation caches (by id
   for relations, by value for the type caches), so a region per `checkExpressionWithContextualType` call would
   need an escape check on every checker cache insert, not a few store sites. Worth a prototype only after 1-2.
   Measure first how many of these types ever enter a value-keyed cache (a counter per cache insert of a type
   created inside such a region).
4. **Parser speculation: ~60-75 MB** (identifiers 25 MB, parameters 6.7, binding elements 5.9, type references
   4.6, node lists 4.3, node arrays 6, parser diagnostics ~6, `ExpressionWithTypeArguments`). Everything a failed
   `tryParse` / `lookAhead` allocated is dead after the rewind: Go discards the nodes and truncates
   `p.diagnostics`; node ids are assigned lazily; `p.identifiers` keeps strings. A checkpoint/rewind on the
   parser thread's arena would reclaim it. That needs a bump arena with mark/reset inside the current chunk
   (bumpalo has none: a small custom arena or a wrapper that records the chunk and offset). The parser's
   side tables must also hold no node pointer created inside the speculation (audit `jsdoc` caches and the
   node-list stack from 113396a). The same mechanism would serve the heritage-clause conversion. Separately, the
   450-950 dropped parses (8.4 MB texts plus their ASTs, run-dependent) are worth tracing on their own: if they
   are duplicate prefetch parses, avoiding them also saves parse time.
5. **Binder flow nodes: ~20 MB** (`FlowNode` 16, `FlowList` 6.6): labels and conditions that
   `finishFlowLabel` / `createFlowCondition` discard (unreachable or single-antecedent labels). The reasoning is
   local to the binder (a label is referenced only from the binder's state until it is finished), so a free
   list or a lazily allocated label is easy. It is small.
6. **Small, each < 10 MB:** `KnownSymlinks::process_resolution` path strings and directory links (8.3 MB, 96%:
   built per resolution, then dropped when the key is already present), node builder contexts and trackers for
   diagnostic text (~10 MB incl. heap), `ExportCollision` (3.3 MB), the declaration arrays left behind by
   `append_declarations` growth (2.2 MB), `WideningContext` + its `GoMap` (3 MB).

Order for the next pass: 2 then 1 (the escape bit is shared; together up to ~380 MB, ~6% of the single-checker peak),
then 4 (independent, parser-only), then 5. Item 3 needs the measurement described there first.
