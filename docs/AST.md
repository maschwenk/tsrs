# `tsrs_ast` API contract

This is the contract between the AST crate and everything that consumes it (scanner,
parser, binder, checker, compiler). It is written before the crate exists so that
consumers can be ported in parallel. The `ast-core` agent implements it and keeps this
file accurate; if reality has to deviate, this file is updated in the same commit.

Everything follows `docs/PORTING.md`. The Go originals are `ts-ref/tsc/internal/ast/*.go`.

## Node

```rust
pub struct Node {
    pub kind: Kind,                       // never changes
    pub flags: Cell<NodeFlags>,
    pub loc: Cell<TextRange>,
    pub parent: Cell<Option<P<Node>>>,
    id: AtomicU64,                        // lazily assigned, see get_node_id
    pub data: NodeData,
}

pub enum NodeData {
    Token,                                // tokens/keywords with no payload (Go `Token`)
    Identifier(&'static Identifier),
    BinaryExpression(&'static BinaryExpression),
    // … one variant per Go node data struct, payload is `&'static <GoStructName>`
}
```

Nodes are always handled as `P<Node>`. A Go function that takes a typed data pointer
(`*ast.BinaryExpression`, `*ast.SourceFile`) takes `P<Node>` in Rust, **except**
`SourceFile`, which is passed as `P<SourceFile>`-like handle — see below.

Convenience methods on `Node` (all `&self`):

- `pos() -> i32`, `end() -> i32`, `loc() -> TextRange`, `set_loc(TextRange)`
- `flags() -> NodeFlags`, `set_flags(NodeFlags)`
- `parent() -> Option<P<Node>>`, `set_parent(Option<P<Node>>)`
  (Go `node.Parent.Kind` where Go assumes non-nil -> `node.parent().unwrap().kind`)
- `as_<snake_case struct name>() -> &'static <Struct>` for every node struct, e.g.
  `as_binary_expression()`, `as_identifier()`, `as_source_file()`. Panics on kind mismatch, like the Go type assertion.
- Every generic accessor Go defines on `*Node` exists with the snake_case name:
  `name()`, `modifiers()`, `expression()`, `type_node()` (Go `Type()`), `type_parameters()`, `parameters()`,
  `body()`, `initializer()`, `text()`, `symbol()`, `locals()`, `statements()`, `members()`, `arguments()`, …
  Return types follow PORTING.md: nil-able -> `Option<…>`; node slices -> `&'static [P<Node>]`.
- Base-struct accessors keep their Go names: `declaration_data()`, `exportable_data()`, `locals_container_data()`,
  `function_like_data()`, `class_like_data()`, `body_data()`, `flow_node_data()`, `literal_like_data()`, … returning
  `Option<&'static XxxBase>`.
- `for_each_child(&self, f: &mut dyn FnMut(P<Node>) -> bool) -> bool` — same contract as Go `ForEachChild`
  (returning `true` stops and propagates `true`).

Free functions mirror Go: `is_binary_expression(node: P<Node>) -> bool`, `get_node_id(node) -> NodeId`,
`get_symbol_id(symbol) -> SymbolId`, everything from `utilities.go` under its snake_case name.

## Node data structs

One struct per Go struct, same name. Fields are the snake_case of the Go fields
(`Left` -> `left`, `OperatorToken` -> `operator_token`, `Type` -> `type_`).

- Child node: `P<Node>`; optional child (marked optional in `tools/scripts/tsc/ast.json` / `// Optional` in Go): `Option<P<Node>>`.
- Child list: `P<NodeList>` / `Option<P<NodeList>>`. Modifiers: `Option<P<ModifierList>>`.
- Strings: `&'static str`. Flags/numbers: plain values.
- Fields that Go mutates after the node is created (by parser fix-ups, reparser, binder, or lazily) are wrapped in
  `Cell<…>`/`RefCell<…>`; AST.md lists them. Everything else is a plain immutable field.
- Embedded Go base structs are fields named after the base in snake_case without the `Base` suffix convention
  change: `DeclarationBase` -> `declaration_base: DeclarationBase`, `LocalsContainerBase` -> `locals_container_base`,
  `FunctionLikeBase` -> `function_like_base`, `FlowNodeBase` -> `flow_node_base`, `ExportableBase` -> `exportable_base`,
  `ModifiersBase` -> `modifiers_base`, `ClassLikeBase` -> `class_like_base`, `BodyBase` -> `body_base`, …
  Bases that carry no data in a type-check-only port (`ExpressionBase`, `StatementBase`, `TypeNodeBase`, `CompositeBase`
  and all subtree-facts machinery) are omitted.
  Consumers should prefer the `Node`-level accessors (`node.symbol()`, `node.locals()`, `node.flow_node()`, …) over
  reaching into base fields.

```rust
pub struct NodeList { pub loc: Cell<TextRange>, pub nodes: &'static [P<Node>] }   // handled as P<NodeList>
pub struct ModifierList { pub list: NodeList, pub modifier_flags: ModifierFlags } // handled as P<ModifierList>; `.nodes()`
```

## SourceFile

`SourceFile` is a node data struct like the others (`node.as_source_file()`), but much Go code holds
`*ast.SourceFile` directly. In Rust that is `P<SourceFile>`; `SourceFile` has a `node: Cell<Option<P<Node>>>` back
pointer set by the factory, exposed as `file.as_node() -> P<Node>`. No other data struct has a back pointer.

## NodeFactory

`NodeFactory` methods mirror Go one-to-one, `&mut self`, same parameter order:
`new_binary_expression(modifiers, left, type_node, operator_token, right) -> P<Node>`,
`new_node_list(nodes: Vec<P<Node>>) -> P<NodeList>` (also accepts `&[P<Node>]` via `new_node_list_from_slice`),
`new_token(kind) -> P<Node>`, `new_identifier(text: &'static str) -> P<Node>`, … `update_*` and `clone`/deep-clone
exist only where the parser/reparser/checker need them.

## Symbols

```rust
pub struct Symbol {
    pub flags: Cell<SymbolFlags>,
    pub check_flags: Cell<CheckFlags>,
    pub name: Cell<&'static str>,
    pub declarations: RefCell<Vec<P<Node>>>,
    pub value_declaration: Cell<Option<P<Node>>>,
    pub members: Cell<Option<P<SymbolTable>>>,
    pub exports: Cell<Option<P<SymbolTable>>>,
    id: AtomicU64,
    pub parent: Cell<Option<P<Symbol>>>,
    pub export_symbol: Cell<Option<P<Symbol>>>,
}
```

Go's `SymbolTable` is a map, i.e. a reference type that is aliased and mutated through several owners. Rust:
`pub struct SymbolTable(RefCell<…map from &'static str to P<Symbol>…>)`, handled as `P<SymbolTable>`, with methods
`get(name) -> Option<P<Symbol>>`, `set(name, symbol)`, `delete(name)`, `len()`, `is_empty()`, `has(name)`,
`for_each(impl FnMut(&'static str, P<Symbol>))`, `entries() -> Vec<(&'static str, P<Symbol>)>`, `values() -> Vec<P<Symbol>>`
(iteration returns a snapshot so callers may mutate while iterating). A Go nil table is `None`.
Iteration order is insertion order (the Go code never relies on map order, but deterministic output matters to us).

## Flags and kinds

`Kind` (enum, `#[repr(i16)]`, with `Kind::FirstKeyword`-style associated consts for range markers), `NodeFlags`,
`ModifierFlags`, `SymbolFlags`, `CheckFlags`, `TokenFlags`, `FunctionFlags`, … are bitflags/enums per PORTING.md.

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
