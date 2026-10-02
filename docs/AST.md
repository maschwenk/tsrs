# `tsrs_ast` API contract

This is the contract between the AST crate and everything that consumes it (scanner,
parser, binder, checker, compiler). The `ast-core` agent implements it and keeps this
file accurate; if reality has to deviate, this file is updated in the same commit.

Everything follows `docs/PORTING.md`. The Go originals are `ts-ref/tsc/internal/ast/*.go`.
Most of the crate is generated: `node tools/gen-ast/gen-ast.ts` writes `src/generated.rs` from
`ts-ref/tools/scripts/tsc/ast.json` (the same schema Go's `ast_generated.go` comes from), and
`node tools/gen-ast/gen-flags.ts` writes `kind.rs` and the flag files. Do not edit generated files.

Import everything with `use tsrs_ast::*;` (or `use tsrs_ast as ast;` and `ast::is_identifier(n)`):
`lib.rs` re-exports every module.

## Node

```rust
pub struct Node {                         // 24 bytes; the data struct is allocated right after it (NodeAlloc)
    header: OwnedCell<NodeHeaderWord>,    // kind (never changes), data tag and parent in one word:
                                          // node.kind(), node.data_tag() (crate), node.parent() / set_parent()
    pub flags: OwnedCell<NodeFlags>,     // OwnedCell = Cell written only by the node's owner, see PORTING.md "Threading"
    pub(crate) id: AtomicU32,             // lazily assigned, see get_node_id (utilities); NodeId is still u64
    pub(crate) loc: OwnedCell<TextRange>, // node.loc() / set_loc()
}

// node.data() -> NodeData: a Copy view of the data struct to match on
#[derive(Clone, Copy)]
pub enum NodeData {
    Token,                                // payload-less: the struct has no Rust fields
    Identifier(&'static Identifier),
    BinaryExpression(&'static BinaryExpression),
    SourceFile(&'static SourceFile),
    // … one variant per Go node data struct (not per Kind: ForInOrOfStatement serves ForIn/ForOf,
    // CaseOrDefaultClause serves Case/Default, TypeAliasDeclaration serves JSTypeAliasDeclaration, …)
    FlowSwitchClauseData(&'static FlowSwitchClauseData),
    FlowReduceLabelData(&'static FlowReduceLabelData),
}
```

Payload-less variants (structs with no Rust fields; `as_x()` still returns `&'static X` of the unit
struct): `Token`, `OmittedExpression`, `KeywordTypeNode`, `ThisTypeNode`, `JsxOpeningFragment`,
`JsxClosingFragment`, `JSDocAllType`. Their nodes allocate only the 24-byte header; every other node is one
arena allocation `NodeAlloc<T> { node: Node, data: T }` (`repr(C)`), and `as_x()` / the generated dispatchers
check `data_tag()` and read the data struct at `offset_of!(NodeAlloc<T>, data)` from the header (Go has the
same single allocation: the data struct embeds `NodeBase`).

Nodes are always handled as `P<Node>`. A Go function that takes a typed data pointer
(`*ast.BinaryExpression`) takes `P<Node>` in Rust. `Node` can only be created by `NodeFactory`, so
`node.as_p() -> P<Node>` turns any `&Node` back into its arena pointer.

Methods on `Node` (all `&self`), hand-written in `ast.rs` unless noted:

- `pos()`, `end()`, `loc()`, `set_loc(TextRange)`, `flags()`, `set_flags(NodeFlags)`, `parent()`,
  `set_parent(Option<P<Node>>)`, `as_p()`/`as_node()`, `kind_string()`, `kind_value()`.
  Go `node.Parent.Kind` (Go assumes non-nil) -> `node.parent().unwrap().kind()`.
- Casts (generated): `as_<snake_case struct name>() -> &'static <Struct>` for every node struct, e.g.
  `as_binary_expression()`, `as_identifier()`, `as_type_assertion()` (struct `TypeAssertion`, kind
  `TypeAssertionExpression`), `as_jsdoc_parameter_or_property_tag()`, `as_source_file()`. Panics on
  mismatch like the Go type assertion. `as_source_file_p() -> P<SourceFile>` for the pointer form.
- Generic accessors (same Go names in snake_case; `Type()` is `type_node()`). Nil-able -> `Option`,
  node slices -> `&'static [P<Node>]` (`&[]` when Go returns nil). Exact return types:
  - `Option<P<Node>>`: `name()`, `expression()`, `type_node()`, `initializer()`, `body()`,
    `label()`, `attributes()`, `module_specifier()`, `import_clause()`, `postfix_token()`,
    `question_token()`, `question_dot_token()`, `type_expression()`, `property_name()`,
    `property_name_or_name()`
  - `P<Node>`: `tag_name()`, `statement()`, `class_name()`
  - `Option<P<NodeList>>`: `parameter_list()`, `argument_list()`, `type_argument_list()`,
    `type_parameter_list()`, `member_list()`, `statement_list()`, `comment_list()`
  - `P<NodeList>`: `children()`, `property_list()`, `element_list()`
  - `&'static [P<Node>]`: `parameters()`, `arguments()`, `type_arguments()`, `type_parameters()`,
    `members()`, `statements()`, `comments()`, `properties()`, `elements()`, `modifier_nodes()`
  - `Option<P<ModifierList>>`: `modifiers()` (generated); `modifier_flags() -> ModifierFlags`;
    `decorators() -> Vec<P<Node>>`
  - `Option<P<Symbol>>`: `symbol()`, `local_symbol()`; `locals() -> Option<P<SymbolTable>>`;
    `flow_node() -> Option<P<FlowNode>>`, `set_flow_node(Option<P<FlowNode>>)` (panics without flow node data, like Go's
    nil dereference), `has_flow_node_data()` (Go `FlowNodeData() != nil`)
  - `text() -> &'static str` (joined texts for JsxNamespacedName/JSDoc text are arena-allocated),
    `raw_text() -> &'static str`, `is_type_only()`, `can_have_statements()`, `is_jsdoc()`,
    `contains(Option<P<Node>>) -> bool`, `iter_children() -> Vec<P<Node>>`
  - `jsdoc(file: Option<&'static SourceFile>) -> &'static [P<Node>]`, `eager_jsdoc(file)`
- Base-struct accessors (generated) return `Option<&'static XxxBase>`: `declaration_data()`,
  `exportable_data()`, `locals_container_data()`, `function_like_data()`, `class_like_data()`,
  `body_data()`, `literal_like_data()`, `template_literal_like_data()`. The flow node is read and written only through
  the node-level accessors above: `Identifier` (hand-written, `identifier.rs`) packs its flow node and its text into
  one word. A parser identifier keeps only its text's length and its file's text index (`register_source_text`;
  `SourceFile.text_index`, `FlowNode.text_index`): the text is the source slice ending at the node's end, so the end
  of such an identifier cannot change (`set_loc` panics). `as_identifier().text()` returns the same slice as before.
- Mutators (Go `node.AsMutable().SetX`; `as_mutable()` exists and returns `&Node`):
  `set_modifiers(Option<P<ModifierList>>)`, `set_type(Option<P<Node>>)`, `set_expression(P<Node>)`,
  `set_initializer(P<Node>)`. Only the kinds the reparser mutates are supported (see the Cell list);
  the other Go cases panic.
- `for_each_child(&self, v: &mut dyn FnMut(P<Node>) -> bool) -> bool` — same contract as Go
  `ForEachChild` (returning `true` stops and propagates `true`). Recursion:
  `fn walk(n: P<Node>) -> bool { … n.for_each_child(&mut |c| walk(c)) }`.
- `visit_each_child(&self, v: &mut NodeVisitor) -> P<Node>`, `clone_node(&self, f: &NodeFactory) -> P<Node>`
  (Go `Clone`; named `clone_node` because `clone` collides with `Clone::clone` on `P`).

Free functions mirror Go (generated): `is_<struct>(node: P<Node>) -> bool` for single-kind structs
(`is_type_assertion`, `is_js_type_alias_declaration` for kind aliases), `is_<kind>` for multi-kind
structs (`is_for_in_statement`, `is_for_of_statement`, `is_case_clause`, `is_object_binding_pattern`,
`is_jsdoc_parameter_tag`, …), `is_token`, `is_keyword_expression`, `is_keyword_type_node`, and the
kind-alias guards on `Kind`: `is_token_kind(kind)`, `is_keyword_kind`, `is_modifier_kind`,
`is_assignment_operator`, `is_binary_operator`, … (Go `IsXxx(kind Kind)` names; `IsJSDocKind` is the
hand-written utilities one). Everything from `utilities.go` is under its snake_case name (agent `ast-util`).

Parent pointers work exactly as in Go: `parent` is a `Cell`, so the parser's
`finishNode`/`overrideParentInImmediateChildren` is
`node.for_each_child(&mut |c| { c.parent.set(Some(node)); false })`, and `set_parent_in_children`
(utilities) walks the same way.

## Node data structs

One struct per Go struct, same name. Fields are the snake_case of the Go fields
(`Left` -> `left`, `OperatorToken` -> `operator_token`, `Type` -> `type_`, Go-private `name` ->
`name`, `JSDocPropertyTags` -> `jsdoc_property_tags`). All fields are `pub`.

Field types:
- Child node: `P<Node>`; nil-able child: `Option<P<Node>>`. A child is nil-able when ast.json marks it
  optional, **or** the Go parser was observed to leave it nil (oracle `tools/oracle/nilfields` over the
  whole test corpus + lib files, parsed as .ts/.tsx/.js; input file `tools/gen-ast/nilable-fields.json`),
  or it is constructed nil by the reparser/checker. Fields widened this way (not optional in ast.json):
  `ArrowFunction.equals_greater_than_token` (the node builder passes nil), `CaseOrDefaultClause.expression`, `ExportAssignment.type_`, `FunctionLikeBase.parameters`,
  `ImportAttribute.name`, `JSDocSeeTag.name_expression`, `JSDocTemplateTag.constraint`, `PropertyAssignment.type_`,
  `PropertySignatureDeclaration.{type_, initializer}`, `ShorthandPropertyAssignment.type_`,
  `TaggedTemplateExpression.question_dot_token`, `TypeAliasDeclaration.type_`.
- Child list: `P<NodeList>` / `Option<P<NodeList>>`. Modifiers: `Option<P<ModifierList>>`.
- Raw lists (Go `[]*Node`, `[]string`): `&'static [P<Node>]`, `&'static [&'static str]`.
- Strings: `&'static str`. Kinds (e.g. `operator`, `token`, `keyword`, `phase_modifier`): `Kind`
  (`Kind::Unknown` for Go's zero value). Flags: `TokenFlags`. `SyntheticExpression.type_` (Go `any`,
  a checker `*Type`): `&'static dyn Any`.
- Go-only bookkeeping fields: `symbol`, `local_symbol` (`Cell<Option<P<Symbol>>>`), `locals`
  (`Cell<Option<P<SymbolTable>>>`), `next_container` (`Cell<Option<P<Node>>>`), `flow_node`,
  `end_flow_node`, `return_flow_node`, `fallthrough_flow_node` (`Cell<Option<P<FlowNode>>>`).

**Cell fields.** Besides the bookkeeping fields above, exactly the fields the parser's reparser
writes after construction are `Cell`s (everything else is a plain immutable field). All of these are
`tsrs_core::OwnedCell` (same API as `Cell`), because checkers read them concurrently (PORTING.md, "Threading"):
`ModifiersBase.modifiers`; `FunctionLikeBase.{type_parameters, parameters, type_, full_signature}`;
`ClassLikeBase.{type_parameters, heritage_clauses}`; `VariableDeclaration.{type_, initializer}`;
`ParameterDeclaration.{type_, question_token}`; `PropertyDeclaration.{type_, initializer}`;
`PropertySignatureDeclaration.type_`; `PropertyAssignment.{type_, initializer}`;
`ShorthandPropertyAssignment.{type_, object_assignment_initializer}`; `ExportAssignment.{type_, expression}`;
`ReturnStatement.expression`; `ParenthesizedExpression.expression`; `BinaryExpression.{type_, right}`;
`TypeAliasDeclaration.{type_parameters, type_}`; `ImportClause.phase_modifier`;
`ExpressionWithTypeArguments.type_arguments`, `HeritageClause.types`; and `LiteralLikeNodeBase.token_flags`
(the checker's node builder flips `SingleQuote` on a cloned string literal).
`NodeList.nodes` is immutable. Where Go appends to an existing list in place (reparser.go `@implements`:
`implementsClause.AsHeritageClause().Types.Nodes = append(...)` and `class.HeritageClauses.Nodes = append(...)`),
the reparser builds a new list with the old list's `loc` and stores it in the owning Cell
(`heritage_clause.set_types(...)`, `class.heritage_clauses.set(...)`). A Go `*NodeList` is only ever held by its
owning node, so nothing else can observe the difference.

**Getter methods — prefer these.** Every data struct (and every base struct) has one getter per Go
field it has, *including fields promoted from embedded bases* (Go-style: `decl.body()` works on
`FunctionDeclaration` although the field lives in `body_base`), returning the value with any `Cell`
unwrapped: `bin.left()`, `bin.right()`, `func.type_()`, `func.parameters()`, `func.symbol()`,
`class.members()`. Cell fields also get `set_<field>(value)` (`set_type`, `set_right`,
`set_flow_node`, `set_return_flow_node`, …). Direct field access (`bin.left`) works for non-Cell
own fields; Cell fields need `.get()`/`.set()`.

**Embedded bases.** Go bases that carry Rust data are embedded as fields named after the base in
snake_case: `declaration_base: DeclarationBase {symbol}`, `exportable_base: ExportableBase {local_symbol}`,
`modifiers_base: ModifiersBase {modifiers}`, `locals_container_base: LocalsContainerBase {locals, next_container}`,
`flow_node_base: FlowNodeBase {flow_node}`, `function_like_base: FunctionLikeBase {locals_container_base,
type_parameters, parameters, type_, full_signature}`, `body_base: BodyBase {asterisk_token, body, end_flow_node}`,
`class_like_base: ClassLikeBase {exportable_base, modifiers_base, locals_container_base, name,
type_parameters, heritage_clauses, members}`, `named_member_base: NamedMemberBase {modifiers_base, name,
postfix_token}`, `literal_like_node_base: LiteralLikeNodeBase {text, token_flags}`,
`template_literal_like_node_base: TemplateLiteralLikeNodeBase {literal_like_node_base, raw_text, template_flags}`,
`iteration_statement_base: IterationStatementBase {flow_node_base, statement}`,
`node_with_type_arguments_base: NodeWithTypeArgumentsBase {type_arguments}`,
`union_or_intersection_type_node_base: UnionOrIntersectionTypeNodeBase {types}`,
`jsdoc_tag_base: JSDocTagBase {tag_name, comment}`, `jsdoc_comment_base: JSDocCommentBase {text}`.
Bases without Rust data are transparent (their own bases are inlined into the node struct):
`NodeBase`, `StatementBase` (so statements embed `flow_node_base` directly), `ExpressionBase` and the
other expression bases, `TypeNodeBase`, `TypeSyntaxBase`, `JSDocTypeBase`, `CompositeBase`,
`FunctionLikeWithBodyBase` (nodes embed `function_like_base` and `body_base` directly),
`AccessorDeclarationBase`, `FunctionOrConstructorTypeNodeBase`, `TypeElementBase`, `ClassElementBase`,
`ObjectLiteralElementBase`, `LiteralExpressionBase`.

**Subtree facts** (`subtreefacts.rs`, Go `subtreefacts.go` + every `computeSubtreeFacts`/`propagateSubtreeFacts`):
`SubtreeFacts` bitflags with Go's names minus the `Subtree` prefix (`SubtreeContainsJsx` -> `SubtreeFacts::ContainsJsx`,
`SubtreeExclusionsFunction` -> `SubtreeFacts::ExclusionsFunction`, `SubtreeFactsComputed` -> `SubtreeFacts::Computed`),
`node.subtree_facts()` (Go `Node.SubtreeFacts()`), `contains_object_rest_or_spread(node)`. Go caches the facts of
composite nodes in `CompositeBase`; the port recomputes them on every call (they are a pure function of the finished
subtree), so calling `subtree_facts()` on every node of a deep tree is quadratic. Go uses them to prune the
JSX module-indicator walk (`parseoptions.rs`) and the JS parameter-decorator walk in the compiler; both prune
exactly like Go (the pruning is observable: e.g. a function without a body reports only `ContainsTypeScript`).

```rust
pub struct NodeList { pub loc: Cell<TextRange>, pub nodes: &'static [P<Node>] }   // P<NodeList>; .nodes(), .pos(), .end(), .has_trailing_comma(), .clone_list(f)
pub struct ModifierList { pub list: NodeList, pub modifier_flags: ModifierFlags } // P<ModifierList>; .nodes(), .loc(), .pos(), .end()
```

## SourceFile

`SourceFile` is a node data struct (`node.as_source_file() -> &'static SourceFile`), but Go code that
holds `*ast.SourceFile` uses `P<SourceFile>` (`P::from_static(node.as_source_file())`,
`node.as_source_file_p()`, `file.as_p()`). It has a back pointer set by the factory:
`file.as_node() -> P<Node>`. No other data struct has one.

Fields set by `new_source_file` are plain (`statements: P<NodeList>`, `end_of_file_token: P<Node>`,
private `text`, `parse_options`); everything the parser/binder/program set later is a `pub` `Cell`
(`language_variant`, `script_kind`, `is_declaration_file`, `imports`, `module_augmentations`,
`ambient_module_names`, `comment_directives`, `pragmas`, `referenced_files`,
`type_reference_directives`, `lib_reference_directives`, `check_js_directive`, `node_count`,
`text_count`, `identifier_count`, `reparsed_clones`, `common_js_module_indicator`,
`external_module_indicator`, `symbol_count`, `pattern_ambient_modules`, `global_exports`, the three
diagnostics lists and `bind_diagnostics`) with a same-named getter. Slices are
`Cell<&'static [T]>`; `set_diagnostics(&[P<Diagnostic>])` etc. copy into the arena. Other methods:
`text()`, `file_name() -> &str`, `path() -> &Path`, `parse_options() -> &SourceFileParseOptions`,
`ecma_line_map() -> &'static [TextPos]`, `get_position_map()`, `has_identifier(&str)`, `is_js()`,
`bind_once(impl FnOnce())`, `is_bound()`, `set_jsdoc_cache(map)`, `set_has_lazy_jsdoc(bool)`,
`symbol()`, `locals()`. Lazy JSDoc: the parser registers `set_parse_jsdoc_for_node(fn(&'static SourceFile, P<Node>) -> Vec<P<Node>>)`.
Content mappers are not ported (`is_content_mapped()` is always false, `original_text()` is `text()`).

`SourceFileParseOptions { file_name: String, path: Path, external_module_indicator_options }` (Clone).

## NodeFactory

`NodeFactory` (`Default`, `new(hooks)`) methods mirror Go one-to-one, `&self`, same parameter order:
`new_binary_expression(modifiers, left, type_node, operator_token, right) -> P<Node>`,
`new_token(kind)`, `new_modifier(kind)`, `new_identifier(text: &'static str)`,
`new_for_in_or_of_statement(kind, await_modifier, initializer, expression, statement)`,
`new_js_type_alias_declaration(...)` (one constructor per kind alias), `new_source_file(opts, text, statements, eof)`,
`new_node_list(Vec<P<Node>>) -> P<NodeList>` (also `new_node_list_from_slice(&[P<Node>])`, and
`new_node_list_from_static(&'static [P<Node>])` which keeps slice identity), `new_modifier_list(Vec<P<Node>>)`.
Rules: a parameter's type is the storage field's type (so e.g. `new_arrow_function(..., body: Option<P<Node>>)`
because `BodyBase.body` is nil-able); Go `Kind` params come first as in Go; `NodeFlags` params
(`new_variable_declaration_list(declarations, flags)`, `new_property_access_expression(expr, qdot, name, flags)`)
set `node.flags`; raw-list params are `&'static [T]` (allocate with `alloc_slice`/`alloc_vec`); token-flag
params are masked like Go. Where Go constructs a node with nil for a non-nil-able field and assigns it right
after (reparser: `NewFunctionDeclaration(..., nil params, ...)`), pass a placeholder and set it.
`update_<struct>(node: P<Node>, …same params…)` exists for every node with children (needed by deep clone).
`node_count()`, `text_count()`, `deep_clone_node(Option<P<Node>>)`, `deep_clone_reparse*`.

A `NodeFactory` value is a **handle**, like Go's `*ast.NodeFactory`: every method takes `&self`, and `clone()`
returns another handle to the same factory (hooks `NodeFactoryHooks { on_create, on_update, on_clone }` are
`Rc<dyn Fn>`, the counters are shared). So factory calls nest (`f.new_type_reference_node(f.new_identifier("T"), None)`)
and one factory can be shared by a visitor, the node builder and a printer emit context (Go passes the same pointer).
Node-builder helpers: `create_modifiers_from_modifier_flags(flags, |k| f.new_modifier(k))`,
`replace_modifiers(&f, node, modifiers)`, `has_inferred_type(node)`.

## Visitor (visitor.go)

`NodeVisitor { visit: Option<VisitFn>, factory: NodeFactory, hooks: NodeVisitorHooks }` with
`VisitFn = Rc<dyn Fn(&mut NodeVisitor, P<Node>) -> Option<P<Node>>>` (the callback gets the visitor back,
so it may re-enter it). Hooks are `Rc<dyn Fn(Option<…>, &mut NodeVisitor) -> Option<…>>`. Exported Go
methods: `visit_node`, `visit_nodes` (NodeList), `visit_modifiers`, `visit_embedded_statement`,
`visit_slice`, `visit_each_child`, `visit_source_file`; `new_node_visitor(visit, Option<NodeFactory>, hooks)`.
The visitor owns a factory handle: to share an existing factory (Go passes the same pointer), pass `Some(f.clone())`.
`NodeVisitor` is `Clone` (it carries no state besides the callback, factory and hooks, so a copy behaves like Go's
shared pointer). When a visitor returns nil for a non-optional child, Go's `VisitEachChild` stores the nil; the Rust
`visit_each_child` keeps the original child (`required_child`), which only matters for traversals whose result is
discarded (the declaration transformer's side-effect visitors).

## Symbols

```rust
pub struct Symbol {                                  // 40 bytes
    pub flags: OwnedCell<SymbolFlags>,
    pub check_flags: OwnedCell<CheckFlags>,
    pub name: OwnedStrCell,                          // PackedStr: pointer + length in one word
    declarations: OwnedSliceCell<P<Node>>,           // 4-aligned (pointer, u32 length)
    pub(crate) id: AtomicU32,
    parent_or_tables: OwnedCell<SymbolParentWord>,   // parent, or a tail {parent, members, exports, export_symbol,
                                                     // value_declaration}; a bit: value declaration = declarations[0]
}
```

`Symbol: Default`; `Symbol::new(flags, name) -> P<Symbol>`; getters `flags()`, `name()`,
`declarations()`, `value_declaration()`, `members()`, `exports()`, `parent()`,
`export_symbol()` and setters `set_parent()`, `set_members()`, `set_exports()`, `set_export_symbol()`,
`set_value_declaration()`, `set_declarations(&[_])` (copies), `set_declarations_static(&'static [_])` (shares, Go
`s.Declarations = other.Declarations`), `append_declarations()`; `is_external_module()`, `is_static()`, `combined_local_and_export_symbol_flags()`.
Free: `get_source_file_of_symbol`, `symbol_name`, `escape_symbol_name`, … `get_symbol_id` is in utilities.

`pub struct SymbolTable(RefCell<IndexMap<&'static str, P<Symbol>>>)`, handled as `P<SymbolTable>`
(`SymbolTable: Default`, `SymbolTable::new()`, `with_capacity(n)`, `clone_table()` = Go `maps.Clone`),
methods `get(name)` (**on a `P<SymbolTable>` receiver `.get(name)` resolves to `P::get` — write
`table.lookup(name)`, identical semantics**), `set(name, symbol)`, `delete(name)`, `len()`, `is_empty()`, `has(name)`,
`for_each(FnMut(&'static str, P<Symbol>))`, `entries()`, `keys()`, `values()` (iteration returns a
snapshot so callers may mutate while iterating). A Go nil table is `None`. Insertion order.

Internal symbol names: Go's prefix byte `0xFE` is invalid UTF-8, so the port uses `"\u{7f}"` (one byte,
not valid in identifiers). Constants keep Go names: `InternalSymbolNamePrefix`, `InternalSymbolNameCall`,
`InternalSymbolNameMissing`, … plus `InternalSymbolNamePrefixByte: u8 = 0x7f`. Translate Go
`name[0] == '\xFE'` to `name.as_bytes().first() == Some(&InternalSymbolNamePrefixByte)`; `name[1]` checks keep
working byte-for-byte. (Sort order differs from Go only against non-ASCII names.)

## Flow (flow.go)

`FlowFlags` bitflags; `pub struct FlowNode { flags: Cell<FlowFlags>, node: Cell<Option<P<Node>>>,
antecedent: Cell<Option<P<FlowNode>>>, antecedents: Cell<Option<P<FlowList>>> }` (all pub, plus getters);
`pub struct FlowList { pub flow: P<FlowNode>, pub next: Cell<Option<P<FlowList>>> }`; `FlowLabel = FlowNode`.
`new_flow_switch_clause_data(switch_statement, clause_start: usize, clause_end: usize) -> P<Node>` and
`new_flow_reduce_label_data(target, antecedents) -> P<Node>` create `Kind::Unknown` nodes;
`as_flow_switch_clause_data()`, `as_flow_reduce_label_data()`.

## Flags, kinds, ids

`Kind` (enum, `#[repr(i16)]`, `Kind::FirstKeyword`-style associated consts for range markers,
`Kind::from_i16`, `Kind::Count`), `NodeFlags`, `ModifierFlags`, `SymbolFlags`, `CheckFlags`,
`TokenFlags` (i32), `FunctionFlags` (+ `get_function_flags(Option<P<Node>>)`), `FlowFlags` are
bitflags/enums per PORTING.md (`XxxFlags::None` is the empty set). `NodeId(pub u64)`, `SymbolId(pub u64)`.
`get_resolution_mode_override(attributes: Option<P<Node>>, grammar_error_on_node) -> Option<ResolutionMode>` (Go's
`(*ImportAttributesNode).GetResolutionModeOverride`, also `ImportAttributes::get_resolution_mode_override`).
`AccessKind`, `CommentDirective{loc, kind}`, `CommentRange{text_range, kind, has_trailing_new_line}`,
`FileReference`, `Pragma`, `PragmaArgument`, `PatternAmbientModule`, `CheckJsDirective`,
`SourceFileMetaData` are plain structs as in Go (embedded `core.TextRange` becomes a `text_range` field
plus `pos()`/`end()`).

Language service token cache: `SourceFile::get_or_create_token(kind, pos, end, parent, flags)` (Go `GetOrCreateToken`,
keyed by `TokenCacheKey { parent, loc }` under a mutex; each call creates tokens with a fresh default factory).

Not ported (language service / emit / API only): content mappers, `SourceFileDataKey`,
`GetNameTable`, `GetDeclarationMap`, `Hash`.

## Diagnostics

`tsrs_diagnostics` (see its crate docs):

- `pub struct Message { code, category, key, text, … }` with getters `code() -> i32`, `category() -> Category`,
  `key()`, `text()`; one `pub static` per message with **exactly the Go identifier**:
  Go `diagnostics.Type_0_is_not_assignable_to_type_1` -> Rust `diagnostics::Type_0_is_not_assignable_to_type_1`
  (a `Message`; pass as `&diagnostics::Type_0_is_not_assignable_to_type_1`, type `&'static Message`).
- Message arguments (Go `args ...any`) are passed as `&[&dyn std::fmt::Display]`:
  Go `c.error(node, diagnostics.X, name, count)` -> `self.error(node, &diagnostics::X, &[&name, &count])`; no args -> `&[]`.
  They are formatted into the message text when the diagnostic is created.

`tsrs_ast::Diagnostic` mirrors Go `ast.Diagnostic` (file, loc, code, category, message text, message chain, related
information), handled as `P<Diagnostic>`; constructors as in `diagnostic.go`:
`new_diagnostic(file: Option<P<SourceFile>>, loc: TextRange, message: &'static Message, args: &[&dyn Display]) -> P<Diagnostic>`,
`new_diagnostic_chain(...)`, `add_related_info`, … with Go's names in snake_case.
