// Rust AST code generator: reads ts-ref/tools/scripts/tsc/ast.json (via a copy of the Go generator's
// schema.ts) and writes crates/tsrs_ast/src/generated.rs, the Rust counterpart of ast_generated.go.
//
// Usage: node tools/gen-ast/gen-ast.ts
//
// Emits node data structs, base structs, the NodeData enum, NodeFactory::new_*/update_* constructors,
// Node::as_* casts, is_* predicates, for_each_child/visit_each_child/clone_node, per-struct field getters,
// the Node-level base accessors (declaration_data(), name(), modifiers(), ...) and the kind alias guards.
// Subtree facts, the encoder and TS output are intentionally not generated.

import * as fs from "node:fs";
import * as path from "node:path";
import type { MemberInfo, NodeType } from "./schema.ts";
import { api, kindGuardName } from "./schema.ts";

const root = path.resolve(import.meta.dirname, "../..");
const outPath = path.join(root, "crates/tsrs_ast/src/generated.rs");

// ── naming ───────────────────────────────────────────────────────────────────

const RUST_KEYWORDS = new Set(["type", "in", "as", "ref", "mod", "match", "loop", "override", "static", "const", "struct", "enum", "impl", "fn", "let", "yield", "abstract", "final", "use", "where", "true", "false", "self", "super", "crate", "move", "return", "if", "else", "for", "while", "break", "continue", "pub", "trait", "unsafe", "extern", "dyn", "async", "await", "box", "do", "macro", "priv", "typeof", "unsized", "virtual", "try"]);

function snake(s: string): string {
    s = s.replace(/JSDoc/g, "Jsdoc");
    return s.replace(/([a-z0-9])([A-Z])/g, "$1_$2").replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2").toLowerCase();
}

function ident(s: string): string {
    const n = snake(s);
    return RUST_KEYWORDS.has(n) ? n + "_" : n;
}

function paramName(m: MemberInfo): string {
    return ident(m.goParamName());
}

// ── Cell policy: fields Go mutates after construction ─────────────────────────
// goOnly fields (symbol, locals, flow nodes, ...) are always Cells. The list below is exactly the set of
// fields the parser's reparser writes after construction (MutableNode.SetType/SetExpression/
// SetInitializer/SetModifiers call sites and direct writes in reparser.go). Mirrored in docs/AST.md.
// They are `OwnedCell`s: after binding, only nodes a checker created itself are written (docs/PORTING.md,
// "Threading").

const CELL_EXPLICIT = new Set([
    "ModifiersBase.modifiers",
    "FunctionLikeBase.TypeParameters",
    "FunctionLikeBase.Parameters",
    "FunctionLikeBase.Type",
    "FunctionLikeBase.FullSignature",
    "ClassLikeBase.TypeParameters",
    "ClassLikeBase.HeritageClauses",
    "VariableDeclaration.Type",
    "VariableDeclaration.Initializer",
    "ParameterDeclaration.Type",
    "ParameterDeclaration.QuestionToken",
    "PropertyDeclaration.Type",
    "PropertyDeclaration.Initializer",
    "PropertySignatureDeclaration.Type",
    "PropertyAssignment.Type",
    "PropertyAssignment.Initializer",
    "ShorthandPropertyAssignment.Type",
    "ShorthandPropertyAssignment.ObjectAssignmentInitializer",
    "ExportAssignment.Type",
    "ExportAssignment.Expression",
    "ReturnStatement.Expression",
    "ParenthesizedExpression.Expression",
    "BinaryExpression.Type",
    "BinaryExpression.Right",
    "TypeAliasDeclaration.TypeParameters",
    "TypeAliasDeclaration.Type",
    "ImportClause.PhaseModifier",
    "ExpressionWithTypeArguments.TypeArguments",
    "HeritageClause.Types",
    // Written by the checker's node builder (nodecopy.go: `c.AsStringLiteral().TokenFlags ^= ast.TokenFlagsSingleQuote` on a clone).
    "LiteralLikeNodeBase.TokenFlags",
]);

// Fields that exist in Go but carry no data in a type-check-only port.
const SKIPPED_FIELDS = new Set(["NodeBase.Flags", "CompositeBase.facts"]);

// ── layout model ──────────────────────────────────────────────────────────────

interface Field {
    owner: string; // struct or base that declares it
    name: string; // Go name
    rust: string; // Rust field name
    ty: string; // Rust value type (without Cell)
    cell: boolean;
    member: MemberInfo;
}

interface Layout {
    name: string;
    embeds: string[]; // kept base structs embedded as fields
    fields: Field[]; // own fields
}

function rustType(m: MemberInfo): string {
    if (m.goOnly) {
        switch (m.rawType) {
            case "*Symbol":
                return "Option<P<Symbol>>";
            case "*FlowNode":
                return "Option<P<FlowNode>>";
            case "SymbolTable":
                return "Option<P<SymbolTable>>";
            case "*Node":
                return "Option<P<Node>>";
        }
        throw new Error(`unknown goOnly type ${m.rawType} for ${m.name}`);
    }
    const t = m.type;
    if (t.kind === "list") {
        if (t.listKind === "raw") {
            if (t.elementType.baseKind() === "node") return "&'static [P<Node>]";
            if (t.elementType.kind === "primitive" && t.elementType.name === "string") return "&'static [&'static str]";
            throw new Error(`unsupported raw list for ${m.name}`);
        }
        if (t.listKind === "ModifierList") return "Option<P<ModifierList>>";
        return m.optional ? "Option<P<NodeList>>" : "P<NodeList>";
    }
    switch (t.baseKind()) {
        case "list":
            return m.optional ? "Option<P<NodeList>>" : "P<NodeList>";
        case "node":
            return m.optional ? "Option<P<Node>>" : "P<Node>";
        case "kind":
            return "Kind";
        case "primitive": {
            const name = (t as any).name;
            switch (name) {
                case "bool":
                    return "bool";
                case "string":
                    return "&'static str";
                case "int":
                    return "i32";
                case "TokenFlags":
                case "NodeFlags":
                case "ModifierFlags":
                    return name;
                case "any":
                    return "&'static dyn Any";
            }
            throw new Error(`unknown primitive ${name} for ${m.name}`);
        }
    }
    throw new Error(`unknown type for ${m.name}`);
}

function isCell(owner: string, m: MemberInfo): boolean {
    return m.goOnly || CELL_EXPLICIT.has(`${owner}.${m.name}`);
}

// Fields the Go parser was observed to leave nil although ast.json does not mark them optional
// (tools/oracle/nilfields run over the test corpus + lib files, parsed as .ts/.tsx/.js).
const OBSERVED_NIL = new Set<string>(JSON.parse(fs.readFileSync(path.join(import.meta.dirname, "nilable-fields.json"), "utf8")));
const widenedToOption: string[] = [];
// Constructed with nil by the reparser or checker (then assigned, or left nil on synthetic nodes).
const FORCE_OPTION = new Set(["FunctionLikeBase.Parameters", "TypeAliasDeclaration.Type"]);

function makeField(owner: string, m: MemberInfo): Field {
    let ty = rustType(m);
    if ((ty === "P<Node>" || ty === "P<NodeList>") && (OBSERVED_NIL.has(`${owner}.${m.name}`) || FORCE_OPTION.has(`${owner}.${m.name}`))) {
        ty = `Option<${ty}>`;
        widenedToOption.push(`${owner}.${m.name}`);
    }
    return { owner, name: m.name, rust: ident(m.name), ty, cell: isCell(owner, m), member: m };
}

function baseOwnFields(key: string): Field[] {
    const base = api.getBase(key)!;
    return base.fields.filter(f => !f.noGo && !SKIPPED_FIELDS.has(`${key}.${f.name}`)).map(f => makeField(key, f));
}

function isKeptBase(key: string): boolean {
    return baseOwnFields(key).length > 0;
}

// Bases without own Rust fields are transparent: their own bases are inlined into the embedding struct.
function expandEmbeds(keys: string[], seen: Set<string>, owner: string): string[] {
    const result: string[] = [];
    for (const key of keys) {
        if (seen.has(key)) continue;
        seen.add(key);
        if (isKeptBase(key)) {
            for (const d of allBasesBelow(key)) {
                if (seen.has(d) && isKeptBase(d)) throw new Error(`${owner}: base ${d} embedded twice`);
                seen.add(d);
            }
            result.push(key);
        }
        else {
            result.push(...expandEmbeds(api.getBase(key)!.extendsKeys, seen, owner));
        }
    }
    return result;
}

function allBasesBelow(key: string): string[] {
    const out: string[] = [];
    for (const k of api.getBase(key)!.extendsKeys) {
        out.push(k, ...allBasesBelow(k));
    }
    return out;
}

const baseLayouts = new Map<string, Layout>();
function baseLayout(key: string): Layout {
    let l = baseLayouts.get(key);
    if (!l) {
        l = { name: key, embeds: expandEmbeds(api.getBase(key)!.extendsKeys, new Set(), key), fields: baseOwnFields(key) };
        baseLayouts.set(key, l);
    }
    return l;
}

function nodeOwnMembers(node: NodeType): MemberInfo[] {
    return node.members.filter(m => !m.inherited && !m.isKindParam() && !m.noGo);
}

function nodeLayout(node: NodeType): Layout {
    return {
        name: node.name,
        embeds: expandEmbeds(node.extendsKeys, new Set(), node.name),
        fields: nodeOwnMembers(node).map(m => makeField(node.name, m)),
    };
}

interface FlatField extends Field {
    path: string; // e.g. "function_like_base.locals_container_base.locals"
    depth: number;
}

function flatten(l: Layout, prefix = "", depth = 0): FlatField[] {
    const out: FlatField[] = [];
    for (const f of l.fields) out.push({ ...f, path: prefix + f.rust, depth });
    for (const e of l.embeds) out.push(...flatten(baseLayout(e), `${prefix}${snake(e)}.`, depth + 1));
    return out;
}

// Promoted field set, Go-style: shallowest wins.
function promoted(l: Layout): FlatField[] {
    const all = flatten(l);
    const byName = new Map<string, FlatField>();
    for (const f of all) {
        const prev = byName.get(f.rust);
        if (!prev || f.depth < prev.depth) byName.set(f.rust, f);
    }
    return all.filter(f => byName.get(f.rust) === f);
}

// Path to an embedded base anywhere inside a layout.
function basePath(l: Layout, key: string, prefix = ""): string | undefined {
    for (const e of l.embeds) {
        const p = `${prefix}${snake(e)}`;
        if (e === key) return p;
        const inner = basePath(baseLayout(e), key, p + ".");
        if (inner) return inner;
    }
    return undefined;
}

function isEmptyLayout(l: Layout): boolean {
    return l.embeds.length === 0 && l.fields.length === 0;
}

// ── writer ────────────────────────────────────────────────────────────────────

const out: string[] = [];
function w(s = "") {
    out.push(s);
}

function fieldDecl(f: Field): string {
    return f.cell ? `OwnedCell<${f.ty}>` : f.ty;
}

function defaultValue(ty: string): string {
    if (ty.startsWith("Option<")) return "None";
    if (ty === "Kind") return "Kind::Unknown";
    if (ty === "bool") return "false";
    if (ty === "&'static str") return '""';
    if (ty.startsWith("&'static [")) return "&[]";
    if (ty === "TokenFlags" || ty === "NodeFlags" || ty === "ModifierFlags") return `${ty}::None`;
    throw new Error(`no default for ${ty}`);
}

function flagConst(mask: string): string {
    const m = /^(TokenFlags|NodeFlags|ModifierFlags)(\w+)$/.exec(mask);
    if (!m) throw new Error(`bad mask ${mask}`);
    return `${m[1]}::${m[2]}`;
}

function isStringType(ty: string) {
    return ty === "&'static str" || ty === "&'static [&'static str]";
}

// ── generation ────────────────────────────────────────────────────────────────

const nodes = api.nodes();
const layouts = new Map<string, Layout>();
for (const n of nodes) layouts.set(n.name, nodeLayout(n));

function schemaMembers(node: NodeType): MemberInfo[] {
    return node.members.filter(m => !m.noFactory);
}

function isNodeFlagsMember(m: MemberInfo) {
    return m.type.kind === "primitive" && m.type.name === "NodeFlags";
}

function findFlat(l: Layout, name: string): FlatField {
    const f = promoted(l).find(f => f.name === name);
    if (!f) throw new Error(`${l.name}: no field ${name}`);
    return f;
}

function header() {
    w("// Code generated by tools/gen-ast/gen-ast.ts from ts-ref/tools/scripts/tsc/ast.json. DO NOT EDIT.");
    w();
    w("use std::any::Any;");
    w();
    w("use tsrs_core::{alloc, OwnedCell, P};");
    w();
    w("use crate::ast::*;");
    w("use crate::flow::*;");
    w("use crate::kind::Kind;");
    w("use crate::nodeflags::NodeFlags;");
    w("use crate::symbol::{Symbol, SymbolTable};");
    w("use crate::tokenflags::TokenFlags;");
    w("use crate::visitor::{same_map_nodes, NodeVisitor};");
    w();
}

function genBaseStructs() {
    w("// ── Base structs ──────────────────────────────────────────────────────────");
    w();
    for (const b of api.bases()) {
        if (!isKeptBase(b.key)) continue;
        const l = baseLayout(b.key);
        w(`pub struct ${b.key} {`);
        for (const e of l.embeds) w(`    pub ${snake(e)}: ${e},`);
        for (const f of l.fields) w(`    pub ${f.rust}: ${fieldDecl(f)},`);
        w("}");
        w();
        genGetters(l);
    }
}

function genGetters(l: Layout) {
    const fields = promoted(l);
    if (fields.length === 0) return;
    w(`impl ${l.name} {`);
    for (const f of fields) {
        const fnName = f.rust;
        w(`    #[inline]`);
        w(`    pub fn ${fnName}(&self) -> ${f.ty} {`);
        w(`        self.${f.path}${f.cell ? ".get()" : ""}`);
        w(`    }`);
        if (f.cell) {
            w(`    #[inline]`);
            w(`    pub fn set_${f.rust.replace(/_$/, "")}(&self, value: ${f.ty}) {`);
            w(`        self.${f.path}.set(value)`);
            w(`    }`);
        }
    }
    w("}");
    w();
}

function genStruct(node: NodeType) {
    const l = layouts.get(node.name)!;
    if (isEmptyLayout(l)) {
        w(`pub struct ${node.name};`);
        w();
        return;
    }
    w(`pub struct ${node.name} {`);
    for (const e of l.embeds) w(`    pub ${snake(e)}: ${e},`);
    for (const f of l.fields) w(`    pub ${f.rust}: ${fieldDecl(f)},`);
    w("}");
    w();
    genGetters(l);
}

// Builds the struct literal for a layout, pulling values from factory params by Go field name.
function structLiteral(l: Layout, values: Map<string, string>, indent: string): string {
    if (isEmptyLayout(l)) return l.name;
    const lines: string[] = [`${l.name} {`];
    for (const e of l.embeds) {
        lines.push(`${indent}    ${snake(e)}: ${structLiteral(baseLayout(e), values, indent + "    ")},`);
    }
    for (const f of l.fields) {
        const v = values.get(f.name) ?? defaultValue(f.ty);
        lines.push(`${indent}    ${f.rust}: ${f.cell ? `OwnedCell::new(${v})` : v},`);
    }
    lines.push(`${indent}}`);
    return lines.join("\n");
}

// Factory parameter types are the storage field types (an inherited member may narrow the Go type, but
// the storage in the base is what Rust stores and returns).
function factoryParams(node: NodeType): { m: MemberInfo; name: string; ty: string; }[] {
    const l = layouts.get(node.name)!;
    return schemaMembers(node).map(m => {
        if (m.isKindParam()) return { m, name: "kind", ty: "Kind" };
        if (isNodeFlagsMember(m)) return { m, name: paramName(m), ty: "NodeFlags" };
        return { m, name: paramName(m), ty: findFlat(l, m.name).ty };
    });
}

function hasTextContent(node: NodeType): boolean {
    return schemaMembers(node).some(m => !m.isKindParam() && !m.goOnly && isStringType(rustType(m)));
}

function genNewFactory(node: NodeType) {
    const l = layouts.get(node.name)!;
    const params = factoryParams(node);
    const kindMember = params.find(p => p.m.isKindParam());
    const flagsMembers = params.filter(p => isNodeFlagsMember(p.m));
    const emit = (fnName: string, kindName: string) => {
        w(`    pub fn ${fnName}(&self${params.map(p => `, ${p.name}: ${p.ty}`).join("")}) -> P<Node> {`);
        const values = new Map<string, string>();
        for (const p of params) {
            if (p.m.isKindParam() || isNodeFlagsMember(p.m)) continue;
            values.set(p.m.name, p.m.bitmask ? `${p.name} & ${flagConst(p.m.bitmask)}` : p.name);
        }
        if (hasTextContent(node)) w(`        self.text_count.set(self.text_count.get() + 1);`);
        const kindArg = kindMember ? "kind" : `Kind::${kindName}`;
        const dataExpr = isEmptyLayout(l) ? `NodeData::${node.name}` : `NodeData::${node.name}(alloc(${structLiteral(l, values, "        ")}))`;
        if (flagsMembers.length > 0) {
            w(`        let node = self.new_node(${kindArg}, ${dataExpr});`);
            for (const f of flagsMembers) {
                if (f.m.bitmask) w(`        node.flags.set(node.flags.get() | (${f.name} & ${flagConst(f.m.bitmask)}));`);
                else w(`        node.flags.set(${f.name});`);
            }
            w(`        node`);
        }
        else {
            w(`        self.new_node(${kindArg}, ${dataExpr})`);
        }
        w(`    }`);
        w();
    };
    emit(`new_${snake(node.name)}`, node.syntaxKindName);
    for (const alias of node.kindAliases) emit(`new_${snake(alias)}`, alias);
}

function diffExpr(ty: string, a: string, b: string): string {
    if (ty.startsWith("&'static [")) return `!same_slice(${a}, ${b})`;
    if (ty === "&'static dyn Any") return `!std::ptr::addr_eq(${a} as *const dyn Any, ${b} as *const dyn Any)`;
    return `${a} != ${b}`;
}

// Value of a factory member read back from existing data (`recv` = data struct, `nodeVar` = the Node).
function memberValue(l: Layout, m: MemberInfo, recv: string, nodeVar: string): string {
    if (m.isKindParam()) return `${nodeVar}.kind`;
    if (isNodeFlagsMember(m)) return `${nodeVar}.flags.get()`;
    return `${recv}.${findFlat(l, m.name).rust}()`;
}

function genUpdateFactory(node: NodeType) {
    const l = layouts.get(node.name)!;
    const params = factoryParams(node);
    const updateParams = params.filter(p => !p.m.isKindParam());
    if (updateParams.length === 0) return;
    if (!updateParams.some(p => p.m.isChild())) return;
    w(`    pub fn update_${snake(node.name)}(&self, node: P<Node>${updateParams.map(p => `, ${p.name}: ${p.ty}`).join("")}) -> P<Node> {`);
    w(`        let data = node.as_${snake(node.name)}();`);
    const cmps = updateParams.map(p => diffExpr(p.ty, p.name, memberValue(l, p.m, "data", "node")));
    w(`        if ${cmps.join(" || ")} {`);
    const newArgs = params.map(p => p.m.isKindParam() ? "node.kind" : p.name).join(", ");
    if (node.kindAliases.length > 0) {
        w(`            let updated = match node.kind {`);
        w(`                Kind::${node.syntaxKindName} => self.new_${snake(node.name)}(${newArgs}),`);
        for (const a of node.kindAliases) w(`                Kind::${a} => self.new_${snake(a)}(${newArgs}),`);
        w(`                _ => panic!("unexpected kind in update_${snake(node.name)}: {:?}", node.kind),`);
        w(`            };`);
        w(`            return update_node(updated, node, &self.hooks);`);
    }
    else {
        w(`            return update_node(self.new_${snake(node.name)}(${newArgs}), node, &self.hooks);`);
    }
    w(`        }`);
    w(`        node`);
    w(`    }`);
    w();
}

function childMembers(node: NodeType) {
    return schemaMembers(node).filter(m => m.isChild());
}

function hasForEachChild(node: NodeType) {
    if (node.handWritten) return true;
    return childMembers(node).length > 0;
}

function genForEachChild(node: NodeType) {
    const l = layouts.get(node.name)!;
    const children = childMembers(node);
    if (children.length === 0) return;
    w(`impl ${node.name} {`);
    w(`    pub fn for_each_child(&self, v: &mut dyn FnMut(P<Node>) -> bool) -> bool {`);
    if (node.handWrittenVisitor) {
        w(`        for_each_child_${snake(node.name)}(self, v)`);
    }
    else {
        const parts = children.map(m => {
            const f = findFlat(l, m.name);
            const access = `self.${f.rust}()`;
            const ty = f.ty;
            if (m.listKind === "raw") return `visit_nodes(v, ${access})`;
            if (m.listKind === "ModifierList") return `visit_modifiers(v, ${access})`;
            if (m.listKind === "NodeList" || m.type.baseKind() === "list") return ty.startsWith("Option") ? `visit_node_list(v, ${access})` : `visit_node_list(v, Some(${access}))`;
            return ty.startsWith("Option") ? `visit(v, ${access})` : `v(${access})`;
        });
        w(`        ${parts.join("\n            || ")}`);
    }
    w(`    }`);
    w(`}`);
    w();
}

function genVisitEachChild(node: NodeType) {
    const l = layouts.get(node.name)!;
    const members = schemaMembers(node);
    const children = members.filter(m => m.isChild());
    if (children.length === 0) return;
    w(`impl ${node.name} {`);
    w(`    pub fn visit_each_child(&self, node: P<Node>, v: &mut NodeVisitor) -> P<Node> {`);
    if (node.handWrittenVisitor) {
        w(`        visit_each_child_${snake(node.name)}(self, node, v)`);
        w(`    }`);
        w(`}`);
        w();
        return;
    }
    const updateMembers = members.filter(m => !m.isKindParam());
    const args: string[] = [];
    for (const m of updateMembers) {
        if (!m.isChild()) {
            args.push(memberValue(l, m, "self", "node"));
            continue;
        }
        const f = findFlat(l, m.name);
        const access = `self.${f.rust}()`;
        const opt = f.ty.startsWith("Option");
        if (m.type.kind === "list" && m.type.listKind === "raw") {
            w(`        let ${paramName(m)} = same_map_nodes(${access}, |n| v.visit_node_hooked(Some(n)).expect("visitor removed a raw list element"));`);
            args.push(paramName(m));
            continue;
        }
        let call: string;
        if (m.visit) call = `v.visit_${snake(m.visit)}_hooked`;
        else if (m.listKind === "ModifierList") call = "v.visit_modifiers_hooked";
        else if (m.listKind === "NodeList" || m.type.baseKind() === "list") call = "v.visit_nodes_hooked";
        else call = "v.visit_node_hooked";
        if (m.visit === "modifiers") call = "v.visit_modifiers_hooked";
        const local = `${paramName(m)}_`;
        if (opt) w(`        let ${local} = ${call}(${access});`);
        else w(`        let ${local} = ${call}(Some(${access})).expect("visitor removed a required child");`);
        args.push(local);
    }
    w(`        v.factory.update_${snake(node.name)}(node, ${args.join(", ")})`);
    w(`    }`);
    w(`}`);
    w();
}

function genClone(node: NodeType) {
    const l = layouts.get(node.name)!;
    const params = factoryParams(node);
    const args = params.map(p => memberValue(l, p.m, "self", "node")).join(", ");
    w(`impl ${node.name} {`);
    w(`    pub fn clone_node(&self, node: P<Node>, f: &NodeFactory) -> P<Node> {`);
    if (node.kindAliases.length > 0) {
        w(`        let updated = match node.kind {`);
        w(`            Kind::${node.syntaxKindName} => f.new_${snake(node.name)}(${args}),`);
        for (const a of node.kindAliases) w(`            Kind::${a} => f.new_${snake(a)}(${args}),`);
        w(`            _ => panic!("unexpected kind in ${node.name}.clone_node: {:?}", node.kind),`);
        w(`        };`);
        w(`        clone_node(updated, node, &f.hooks)`);
    }
    else {
        w(`        clone_node(f.new_${snake(node.name)}(${args}), node, &f.hooks)`);
    }
    w(`    }`);
    w(`}`);
    w();
}

function genIsFunctions(node: NodeType) {
    const kindTypes = node.kindTypes();
    if (node.kindType.kind === "typeParameter") {
        w(`pub fn is_${snake(node.name)}(node: P<Node>) -> bool {`);
        w(`    matches!(node.kind, ${kindTypes.map(k => `Kind::${k.name}`).join(" | ")})`);
        w(`}`);
        w();
        return;
    }
    if (node.isMultiKind()) {
        for (const k of kindTypes) {
            w(`pub fn is_${snake(k.name)}(node: P<Node>) -> bool {`);
            w(`    node.kind == Kind::${k.name}`);
            w(`}`);
            w();
        }
        return;
    }
    w(`pub fn is_${snake(node.name)}(node: P<Node>) -> bool {`);
    w(`    node.kind == Kind::${node.syntaxKindName}`);
    w(`}`);
    w();
    for (const a of node.kindAliases) {
        w(`pub fn is_${snake(a)}(node: P<Node>) -> bool {`);
        w(`    node.kind == Kind::${a}`);
        w(`}`);
        w();
    }
}

// Hand-written node data (not in ast.json, or handWritten there) that still participates in NodeData.
const EXTRA_DATA = ["FlowSwitchClauseData", "FlowReduceLabelData"];

function genNodeDataEnum() {
    w("// ── NodeData ──────────────────────────────────────────────────────────────");
    w();
    w("#[derive(Clone, Copy)]");
    w("pub enum NodeData {");
    for (const n of nodes) {
        if (isEmptyLayout(layouts.get(n.name)!)) w(`    ${n.name},`);
        else w(`    ${n.name}(&'static ${n.name}),`);
    }
    for (const e of EXTRA_DATA) w(`    ${e}(&'static ${e}),`);
    w("}");
    w();
}

function genNodeImpl() {
    w("// ── Node casts and generic dispatch ──────────────────────────────────────");
    w();
    w("impl Node {");
    for (const n of nodes) {
        const fn = `as_${snake(n.name)}`;
        const empty = isEmptyLayout(layouts.get(n.name)!);
        w(`    #[inline]`);
        w(`    pub fn ${fn}(&self) -> &'static ${n.name} {`);
        w(`        match self.data {`);
        w(empty ? `            NodeData::${n.name} => &${n.name},` : `            NodeData::${n.name}(d) => d,`);
        w(`            _ => panic!("${fn} called on {:?}", self.kind),`);
        w(`        }`);
        w(`    }`);
    }
    for (const e of EXTRA_DATA) {
        const fn = `as_${snake(e)}`;
        w(`    #[inline]`);
        w(`    pub fn ${fn}(&self) -> &'static ${e} {`);
        w(`        match self.data {`);
        w(`            NodeData::${e}(d) => d,`);
        w(`            _ => panic!("${fn} called on {:?}", self.kind),`);
        w(`        }`);
        w(`    }`);
    }
    w();

    // for_each_child dispatch
    w(`    pub fn for_each_child(&self, v: &mut dyn FnMut(P<Node>) -> bool) -> bool {`);
    w(`        match self.data {`);
    for (const n of nodes) {
        if (!hasForEachChild(n)) continue;
        w(`            NodeData::${n.name}(d) => d.for_each_child(v),`);
    }
    w(`            _ => false,`);
    w(`        }`);
    w(`    }`);
    w();

    // visit_each_child dispatch
    w(`    pub fn visit_each_child(&self, v: &mut NodeVisitor) -> P<Node> {`);
    w(`        let node = self.as_p();`);
    w(`        match self.data {`);
    for (const n of nodes) {
        if (!hasForEachChild(n)) continue;
        w(`            NodeData::${n.name}(d) => d.visit_each_child(node, v),`);
    }
    w(`            _ => node,`);
    w(`        }`);
    w(`    }`);
    w();

    // clone_node dispatch
    w(`    pub fn clone_node(&self, f: &NodeFactory) -> P<Node> {`);
    w(`        let node = self.as_p();`);
    w(`        match self.data {`);
    for (const n of nodes) {
        if (isEmptyLayout(layouts.get(n.name)!)) w(`            NodeData::${n.name} => ${n.name}.clone_node(node, f),`);
        else w(`            NodeData::${n.name}(d) => d.clone_node(node, f),`);
    }
    w(`            _ => panic!("clone_node: unsupported node data for {:?}", self.kind),`);
    w(`        }`);
    w(`    }`);
    w();

    // Name() / Modifiers()
    const withField = (fieldName: string) =>
        nodes.filter(n => n.name !== "SourceFile" && promoted(layouts.get(n.name)!).some(f => f.name === fieldName));
    w(`    pub fn name(&self) -> Option<P<Node>> {`);
    w(`        match self.data {`);
    for (const n of withField("name")) {
        const f = findFlat(layouts.get(n.name)!, "name");
        w(`            NodeData::${n.name}(d) => ${f.ty.startsWith("Option") ? `d.${f.rust}()` : `Some(d.${f.rust}())`},`);
    }
    w(`            _ => None,`);
    w(`        }`);
    w(`    }`);
    w();
    w(`    pub fn modifiers(&self) -> Option<P<ModifierList>> {`);
    w(`        match self.data {`);
    for (const n of withField("modifiers")) w(`            NodeData::${n.name}(d) => d.modifiers(),`);
    w(`            _ => None,`);
    w(`        }`);
    w(`    }`);
    w();
    w(`    pub(crate) fn set_modifiers_data(&self, modifiers: Option<P<ModifierList>>) {`);
    w(`        match self.data {`);
    for (const n of withField("modifiers")) w(`            NodeData::${n.name}(d) => d.set_modifiers(modifiers),`);
    w(`            _ => {}`);
    w(`        }`);
    w(`    }`);
    w();

    const baseAccessors: [string, string][] = [
        ["flow_node_data", "FlowNodeBase"],
        ["declaration_data", "DeclarationBase"],
        ["exportable_data", "ExportableBase"],
        ["locals_container_data", "LocalsContainerBase"],
        ["function_like_data", "FunctionLikeBase"],
        ["class_like_data", "ClassLikeBase"],
        ["body_data", "BodyBase"],
        ["literal_like_data", "LiteralLikeNodeBase"],
        ["template_literal_like_data", "TemplateLiteralLikeNodeBase"],
    ];
    for (const [fn, base] of baseAccessors) {
        w(`    pub fn ${fn}(&self) -> Option<&'static ${base}> {`);
        w(`        match self.data {`);
        for (const n of nodes) {
            const p = basePath(layouts.get(n.name)!, base);
            if (p) w(`            NodeData::${n.name}(d) => Some(&d.${p}),`);
        }
        w(`            _ => None,`);
        w(`        }`);
        w(`    }`);
        w();
    }
    w("}");
    w();
}

function genFactory() {
    w("// ── NodeFactory constructors ─────────────────────────────────────────────");
    w();
    w("impl NodeFactory {");
    for (const n of nodes) {
        if (n.handWritten) continue;
        genNewFactory(n);
        genUpdateFactory(n);
    }
    w("}");
    w();
}

function genKindAliasGuards() {
    w("// ── Kind alias guards ────────────────────────────────────────────────────");
    w();
    const skip = new Set(["isJSDocKind"]);
    for (const g of api.kindGuards()) {
        const tsName = kindGuardName(g.aliasName);
        if (skip.has(tsName)) continue;
        const fn = snake(tsName);
        w(`pub fn ${fn}(kind: Kind) -> bool {`);
        if (g.type === "range") {
            w(`    kind >= Kind::${g.first} && kind <= Kind::${g.last}`);
        }
        else {
            const expanded = api.expandKindAliasMembers(g.aliasName);
            w(`    matches!(kind, ${expanded.map(k => `Kind::${k.name}`).join(" | ")})`);
        }
        w(`}`);
        w();
    }
}

header();
genBaseStructs();
w("// ── Node data structs ─────────────────────────────────────────────────────");
w();
for (const n of nodes) {
    if (n.handWritten) continue;
    genStruct(n);
}
genNodeDataEnum();
genNodeImpl();
genFactory();
for (const n of nodes) {
    if (n.handWritten) continue;
    genForEachChild(n);
    genVisitEachChild(n);
    genClone(n);
}
w("// ── is_* predicates ──────────────────────────────────────────────────────");
w();
for (const n of nodes) genIsFunctions(n);
genKindAliasGuards();

fs.writeFileSync(outPath, out.join("\n"));
console.log(`Widened to Option (observed nil): ${[...new Set(widenedToOption)].sort().join(", ")}`);
console.log(`Wrote ${outPath}`);
