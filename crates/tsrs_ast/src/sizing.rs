// TOOL, not for main (branch notes/compact-ast-sizing): counts the syntax tree of the parsed program for
// notes/mem-compact-ast-sizing.md.
//
// `TSRS_AST_SIZING=<file>` makes the type-check pass (`Program::get_semantic_diagnostics(None)`) append TSV rows to
// <file> at two points: `pre` (after load, bind and the leaf classification, before the checkers start: every file)
// and `post` (after the pass: every file that is still allocated, i.e. all but the freed check leaves, with the lazy
// member lists the checkers forced). Nothing is forced or allocated in the arenas: a lazy member list that is not
// parsed yet is counted as its record, JSDoc as far as it is in a file's cache. The walk is `for_each_child` from
// each `SourceFile` node plus every JSDoc node in the file's cache; `visit_node_list` / `visit_modifiers` report each
// list they visit while a walk runs (`note_list`). Bytes are arena bytes: `NodeAlloc<T>` / `NodeAllocRare<T, R>`
// (8-aligned, padded to 8), 16-byte `NodeList`s, 24-byte `ModifierList`s, 4 bytes per list element.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::io::Write as _;
use std::mem::{align_of, size_of};
use std::sync::atomic::Ordering;

use rustc_hash::FxHashSet;
use tsrs_core::P;

use crate::ast::{NodeAlloc, NodeAllocRare, NodeRareTail};
use crate::generated::NodeDataTag;
use crate::lazylist::{LazyNodeList, DONE};
use crate::*;

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static LISTS: RefCell<Vec<ListSeen>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Plain,
    Modifiers,
    LazyForced,
    LazyUnforced,
}

const LIST_KINDS: [(ListKind, &str); 4] =
    [(ListKind::Plain, "plain"), (ListKind::Modifiers, "modifiers"), (ListKind::LazyForced, "lazy_forced"), (ListKind::LazyUnforced, "lazy_unforced")];

#[derive(Clone, Copy)]
struct ListSeen {
    kind: ListKind,
    len: usize,
}

/// Whether a sizing walk runs on this thread (`visit_node_list` / `visit_modifiers` then call `note_list`).
#[inline]
pub(crate) fn active() -> bool {
    ACTIVE.with(Cell::get)
}

/// Records a list the walk visits and returns its nodes; `None` for a lazy member list that is not parsed yet (it is
/// not forced).
pub(crate) fn note_list(list: &NodeList, modifiers: bool) -> Option<&'static [P<Node>]> {
    let lazy = list.lazy_record();
    if let Some(rec) = lazy {
        if rec.state() != DONE {
            LISTS.with(|l| l.borrow_mut().push(ListSeen { kind: ListKind::LazyUnforced, len: 0 }));
            return None;
        }
    }
    let nodes = list.nodes();
    let kind = if modifiers {
        ListKind::Modifiers
    } else if lazy.is_some() {
        ListKind::LazyForced
    } else {
        ListKind::Plain
    };
    LISTS.with(|l| l.borrow_mut().push(ListSeen { kind, len: nodes.len() }));
    Some(nodes)
}

macro_rules! layouts {
    (plain: [$($p:ident),* $(,)?], rare: [$($r:ident),* $(,)?], empty: [$($e:ident),* $(,)?] $(,)?) => {
        /// Arena bytes of the node's allocation (header, data struct, rare tail), as `P::new` lays it out.
        fn alloc_size(n: &Node) -> usize {
            match n.data_tag() {
                NodeDataTag::Identifier => crate::identifier::identifier_alloc_size(n),
                $(NodeDataTag::$p => size_of::<NodeAlloc<$p>>(),)*
                $(NodeDataTag::$r => {
                    if n.has_rare_tail() {
                        size_of::<NodeAllocRare<$r, <$r as NodeRareTail>::Rare>>()
                    } else {
                        size_of::<NodeAlloc<$r>>()
                    }
                })*
                $(NodeDataTag::$e => size_of::<Node>(),)*
            }
        }

        fn tag_name(t: usize) -> &'static str {
            if t == NodeDataTag::Identifier as usize {
                return "Identifier";
            }
            $(if t == NodeDataTag::$p as usize { return stringify!($p); })*
            $(if t == NodeDataTag::$r as usize { return stringify!($r); })*
            $(if t == NodeDataTag::$e as usize { return stringify!($e); })*
            "?"
        }

        /// (data struct, `NodeAlloc` bytes, data struct bytes, data struct alignment, `NodeAllocRare` bytes or 0).
        fn layout_rows() -> Vec<(&'static str, usize, usize, usize, usize)> {
            vec![
                ("Identifier", size_of::<NodeAlloc<Identifier>>(), size_of::<Identifier>(), align_of::<Identifier>(), crate::identifier::IDENTIFIER_WITH_TEXT_SIZE),
                $((stringify!($p), size_of::<NodeAlloc<$p>>(), size_of::<$p>(), align_of::<$p>(), 0),)*
                $((stringify!($r), size_of::<NodeAlloc<$r>>(), size_of::<$r>(), align_of::<$r>(), size_of::<NodeAllocRare<$r, <$r as NodeRareTail>::Rare>>()),)*
                $((stringify!($e), size_of::<Node>(), 0, 1, 0),)*
            ]
        }
    };
}

layouts! {
    plain: [
        PrivateIdentifier, QualifiedName, ComputedPropertyName, Decorator, EmptyStatement, IfStatement, DoStatement,
        WhileStatement, ForStatement, ForInOrOfStatement, BreakStatement, ContinueStatement, ReturnStatement,
        WithStatement, SwitchStatement, CaseBlock, CaseOrDefaultClause, ThrowStatement, TryStatement, CatchClause,
        DebuggerStatement, LabeledStatement, ExpressionStatement, Block, VariableStatement, VariableDeclarationList,
        BindingPattern, MissingDeclaration, FunctionDeclaration, ClassDeclaration, ClassExpression, HeritageClause,
        InterfaceDeclaration, TypeAliasDeclaration, EnumMember, EnumDeclaration, ModuleBlock, NotEmittedStatement,
        NotEmittedTypeElement, ExternalModuleReference, NamespaceImport, NamedImports, ExportAssignment,
        NamespaceExportDeclaration, NamespaceExport, NamedExports, ExportSpecifier, CallSignatureDeclaration,
        ConstructSignatureDeclaration, ConstructorDeclaration, GetAccessorDeclaration, SetAccessorDeclaration,
        IndexSignatureDeclaration, MethodSignatureDeclaration, MethodDeclaration, PropertyDeclaration,
        SemicolonClassElement, ClassStaticBlockDeclaration, KeywordExpression, StringLiteral, NumericLiteral,
        BigIntLiteral, RegularExpressionLiteral, NoSubstitutionTemplateLiteral, BinaryExpression,
        PrefixUnaryExpression, PostfixUnaryExpression, YieldExpression, ArrowFunction, FunctionExpression,
        AsExpression, SatisfiesExpression, ConditionalExpression, MetaProperty, NonNullExpression, SpreadElement,
        TemplateExpression, TemplateSpan, TaggedTemplateExpression, ParenthesizedExpression, ArrayLiteralExpression,
        ObjectLiteralExpression, SpreadAssignment, PropertyAssignment, ShorthandPropertyAssignment, DeleteExpression,
        TypeOfExpression, VoidExpression, AwaitExpression, TypeAssertion, UnionTypeNode, IntersectionTypeNode,
        ConditionalTypeNode, TypeOperatorNode, InferTypeNode, ArrayTypeNode, IndexedAccessTypeNode,
        TypeReferenceNode, ExpressionWithTypeArguments, LiteralTypeNode, TypePredicateNode, ImportAttribute,
        ImportAttributes, TypeQueryNode, MappedTypeNode, TypeLiteralNode, TupleTypeNode, NamedTupleMember,
        OptionalTypeNode, RestTypeNode, ParenthesizedTypeNode, FunctionTypeNode, ConstructorTypeNode, TemplateHead,
        TemplateMiddle, TemplateTail, TemplateLiteralTypeNode, TemplateLiteralTypeSpan, SyntheticExpression,
        PartiallyEmittedExpression, JsxElement, JsxAttributes, JsxNamespacedName, JsxOpeningElement,
        JsxSelfClosingElement, JsxFragment, JsxAttribute, JsxSpreadAttribute, JsxClosingElement, JsxExpression,
        JsxText, SyntaxList, JSDoc, JSDocTypeExpression, JSDocNonNullableType, JSDocNullableType, JSDocVariadicType,
        JSDocOptionalType, JSDocTypeTag, JSDocUnknownTag, JSDocTemplateTag, JSDocReturnTag, JSDocPublicTag,
        JSDocPrivateTag, JSDocProtectedTag, JSDocReadonlyTag, JSDocOverrideTag, JSDocDeprecatedTag, JSDocSeeTag,
        JSDocImplementsTag, JSDocAugmentsTag, JSDocSatisfiesTag, JSDocThrowsTag, JSDocThisTag, JSDocImportTag,
        JSDocCallbackTag, JSDocOverloadTag, JSDocTypedefTag, JSDocSignature, JSDocNameReference, SourceFile,
        ModuleDeclaration, ImportEqualsDeclaration, ExportDeclaration, ImportTypeNode, ImportClause, JSDocText,
        JSDocLink, JSDocLinkPlain, JSDocLinkCode, TypeParameterDeclaration, SyntheticReferenceExpression,
        JSDocTypeLiteral, JSDocParameterOrPropertyTag, FlowSwitchClauseData, FlowReduceLabelData,
    ],
    rare: [
        VariableDeclaration, ParameterDeclaration, BindingElement, ImportDeclaration, PropertySignatureDeclaration,
        PropertyAccessExpression, ElementAccessExpression, CallExpression, NewExpression, ImportSpecifier,
    ],
    empty: [Token, OmittedExpression, KeywordTypeNode, ThisTypeNode, JsxOpeningFragment, JsxClosingFragment, JSDocAllType],
}

// Identifier roles: which field of which parent holds the identifier.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Role {
    Expr,
    PropAccessExpr,
    PropAccessName,
    MemberDeclName,
    ObjectPropName,
    ShorthandName,
    JsxAttrName,
    BindingPropName,
    DeclName,
    Specifier,
    ImportBinding,
    TypeRefName,
    HeritageName,
    TypeQueryName,
    QualifiedOther,
    JsxTagName,
    Label,
    MetaName,
    TypePredicateParam,
    JSDoc,
    Count,
}

const ROLE_NAMES: [&str; Role::Count as usize] = [
    "expr",
    "prop_access_expr",
    "prop_access_name",
    "member_decl_name",
    "object_prop_name",
    "shorthand_name",
    "jsx_attr_name",
    "binding_prop_name",
    "decl_name",
    "import_export_specifier",
    "import_binding",
    "type_ref_name",
    "heritage_name",
    "type_query_name",
    "qualified_other",
    "jsx_tag_name",
    "label",
    "meta_property_name",
    "type_predicate_param",
    "jsdoc",
];

fn role(child: P<Node>, parent: Option<P<Node>>, top: Kind, in_jsdoc: bool) -> Role {
    if in_jsdoc {
        return Role::JSDoc;
    }
    let Some(p) = parent else {
        return Role::Expr;
    };
    let is_name = || p.name() == Some(child);
    match p.kind() {
        Kind::PropertyAccessExpression => {
            if is_name() {
                Role::PropAccessName
            } else {
                Role::PropAccessExpr
            }
        }
        Kind::QualifiedName => match top {
            Kind::TypeReference | Kind::ImportType => Role::TypeRefName,
            Kind::ExpressionWithTypeArguments => Role::HeritageName,
            Kind::TypeQuery => Role::TypeQueryName,
            _ => Role::QualifiedOther,
        },
        Kind::TypeReference => Role::TypeRefName,
        Kind::ImportType => Role::TypeRefName,
        Kind::ExpressionWithTypeArguments => Role::HeritageName,
        Kind::TypeQuery => Role::TypeQueryName,
        Kind::ImportSpecifier | Kind::ExportSpecifier => Role::Specifier,
        Kind::ImportClause
        | Kind::NamespaceImport
        | Kind::NamespaceExport
        | Kind::ImportEqualsDeclaration
        | Kind::NamespaceExportDeclaration => {
            if is_name() {
                Role::ImportBinding
            } else {
                Role::Expr
            }
        }
        Kind::PropertySignature
        | Kind::PropertyDeclaration
        | Kind::MethodSignature
        | Kind::MethodDeclaration
        | Kind::GetAccessor
        | Kind::SetAccessor
        | Kind::EnumMember => {
            if is_name() {
                Role::MemberDeclName
            } else {
                Role::Expr
            }
        }
        Kind::PropertyAssignment => {
            if is_name() {
                Role::ObjectPropName
            } else {
                Role::Expr
            }
        }
        Kind::ShorthandPropertyAssignment => {
            if is_name() {
                Role::ShorthandName
            } else {
                Role::Expr
            }
        }
        Kind::JsxAttribute => {
            if is_name() {
                Role::JsxAttrName
            } else {
                Role::Expr
            }
        }
        Kind::BindingElement => {
            if p.property_name() == Some(child) {
                Role::BindingPropName
            } else if is_name() {
                Role::DeclName
            } else {
                Role::Expr
            }
        }
        Kind::VariableDeclaration
        | Kind::Parameter
        | Kind::FunctionDeclaration
        | Kind::FunctionExpression
        | Kind::ClassDeclaration
        | Kind::ClassExpression
        | Kind::InterfaceDeclaration
        | Kind::TypeAliasDeclaration
        | Kind::EnumDeclaration
        | Kind::ModuleDeclaration
        | Kind::TypeParameter
        | Kind::NamedTupleMember => {
            if is_name() {
                Role::DeclName
            } else {
                Role::Expr
            }
        }
        Kind::JsxOpeningElement | Kind::JsxSelfClosingElement | Kind::JsxClosingElement => Role::JsxTagName,
        Kind::LabeledStatement | Kind::BreakStatement | Kind::ContinueStatement => Role::Label,
        Kind::MetaProperty => Role::MetaName,
        Kind::TypePredicate => Role::TypePredicateParam,
        _ => Role::Expr,
    }
}

const CLASSES: [&str; 3] = ["src", "dts", "leaf"];
const KINDS: usize = 512;
const BUCKETS: [&str; 5] = ["0", "1", "2-3", "4-8", ">8"];

fn bucket(len: usize) -> usize {
    match len {
        0 => 0,
        1 => 1,
        2..=3 => 2,
        4..=8 => 3,
        _ => 4,
    }
}

#[derive(Clone, Copy, Default)]
struct KindRow {
    count: u64,
    bytes: u64,
    with_id: u64,
    with_rare: u64,
    in_jsdoc: u64,
}

#[derive(Clone, Copy, Default)]
struct IdentRow {
    count: u64,
    bytes: u64,
    compact: u64,
    stored_borrow: u64,
    stored_copy: u64,
    copy_text_bytes: u64,
    text_bytes: u64,
    with_flow: u64,
    with_id: u64,
}

#[derive(Clone, Copy, Default)]
struct LiteralRow {
    count: u64,
    bytes: u64,
    text_bytes: u64,
    copied: u64,
    copied_bytes: u64,
    raw_copied_bytes: u64,
}

#[derive(Clone, Copy, Default)]
struct ListRow {
    count: u64,
    struct_bytes: u64,
    slice_bytes: u64,
    elements: u64,
}

struct ClassCounts {
    files: u64,
    text_bytes: u64,
    kinds: Vec<KindRow>,
    idents: [IdentRow; Role::Count as usize],
    ident_parent: Vec<[u64; KINDS]>,
    literals: Vec<LiteralRow>,
    lists: [[ListRow; 5]; 4],
    list_owner: Vec<[u64; 5]>,
    jsdoc_cache_entries: u64,
    jsdoc_cache_slice_bytes: u64,
    dups: u64,
    /// Per data tag: (count, bytes, nodes with a rare tail).
    tags: Vec<[u64; 3]>,
    /// Identifier texts: distinct per file, summed over files (count, bytes), all roles and the removable-name roles.
    file_unique: [u64; 2],
    file_unique_names: [u64; 2],
}

impl ClassCounts {
    fn new() -> ClassCounts {
        ClassCounts {
            files: 0,
            text_bytes: 0,
            kinds: vec![KindRow::default(); KINDS],
            idents: [IdentRow::default(); Role::Count as usize],
            ident_parent: vec![[0; KINDS]; Role::Count as usize],
            literals: vec![LiteralRow::default(); KINDS],
            lists: [[ListRow::default(); 5]; 4],
            list_owner: vec![[0; 5]; KINDS],
            jsdoc_cache_entries: 0,
            jsdoc_cache_slice_bytes: 0,
            dups: 0,
            tags: vec![[0; 3]; 256],
            file_unique: [0; 2],
            file_unique_names: [0; 2],
        }
    }
}

/// Visited set over node handles (offset / 8 from the arena base), so a node reachable twice counts once.
struct Seen {
    bits: Vec<u64>,
    other: FxHashSet<usize>,
}

impl Seen {
    fn new() -> Seen {
        Seen { bits: vec![0; 1 << 26], other: FxHashSet::default() }
    }

    fn insert(&mut self, n: P<Node>) -> bool {
        let h = n.to_bits() >> 3;
        if h < 1 << 32 {
            let (w, b) = (h >> 6, 1u64 << (h & 63));
            let fresh = self.bits[w] & b == 0;
            self.bits[w] |= b;
            fresh
        } else {
            self.other.insert(h)
        }
    }
}

fn in_text(s: &str, text: &str) -> bool {
    let (a, b) = (s.as_ptr() as usize, text.as_ptr() as usize);
    a >= b && a + s.len() <= b + text.len()
}

struct Frame {
    node: P<Node>,
    parent: Option<P<Node>>,
    top: Kind,
    jsdoc: bool,
}

/// Distinct identifier texts: per file (cleared per file) and program-wide; `names` only for the roles a name-less
/// parent would hold (property-access names, member declaration names, import/export specifier names).
#[derive(Default)]
struct Texts {
    file: FxHashSet<&'static str>,
    file_names: FxHashSet<&'static str>,
    global: FxHashSet<&'static str>,
    global_names: FxHashSet<&'static str>,
}

fn walk(c: &mut ClassCounts, stack: &mut Vec<Frame>, seen: &mut Seen, text: &'static str, texts: &mut Texts) {
    while let Some(f) = stack.pop() {
        let n = f.node;
        if !seen.insert(n) {
            c.dups += 1;
            continue;
        }
        let kind = n.kind();
        let k = kind as usize;
        let size = alloc_size(&n) as u64;
        let has_id = n.id.load(Ordering::Relaxed) != 0;
        let row = &mut c.kinds[k];
        row.count += 1;
        row.bytes += size;
        row.with_id += u64::from(has_id);
        row.with_rare += u64::from(n.has_rare_tail());
        row.in_jsdoc += u64::from(f.jsdoc);
        let tr = &mut c.tags[n.data_tag() as usize];
        tr[0] += 1;
        tr[1] += size;
        tr[2] += u64::from(n.has_rare_tail());
        match n.data_tag() {
            NodeDataTag::Identifier => {
                let r = role(n, f.parent, f.top, f.jsdoc);
                let id = n.as_identifier();
                let t = id.text();
                let ir = &mut c.idents[r as usize];
                ir.count += 1;
                ir.bytes += size;
                ir.text_bytes += t.len() as u64;
                ir.with_id += u64::from(has_id);
                ir.with_flow += u64::from(id.flow_node().is_some());
                if id.is_source_text() {
                    ir.compact += 1;
                } else if in_text(t, text) {
                    ir.stored_borrow += 1;
                } else {
                    ir.stored_copy += 1;
                    ir.copy_text_bytes += t.len() as u64;
                }
                c.ident_parent[r as usize][f.parent.map_or(0, |p| p.kind() as usize)] += 1;
                if texts.file.insert(t) {
                    c.file_unique[0] += 1;
                    c.file_unique[1] += t.len() as u64;
                }
                texts.global.insert(t);
                if matches!(r, Role::PropAccessName | Role::MemberDeclName | Role::Specifier) {
                    if texts.file_names.insert(t) {
                        c.file_unique_names[0] += 1;
                        c.file_unique_names[1] += t.len() as u64;
                    }
                    texts.global_names.insert(t);
                }
            }
            NodeDataTag::PrivateIdentifier => {
                let t = n.as_private_identifier().text();
                let lr = &mut c.literals[k];
                lr.count += 1;
                lr.bytes += size;
                lr.text_bytes += t.len() as u64;
                if !in_text(t, text) {
                    lr.copied += 1;
                    lr.copied_bytes += t.len() as u64;
                }
            }
            _ => {
                if let Some(lit) = n.literal_like_data() {
                    let t = lit.text();
                    let lr = &mut c.literals[k];
                    lr.count += 1;
                    lr.bytes += size;
                    lr.text_bytes += t.len() as u64;
                    if !in_text(t, text) {
                        lr.copied += 1;
                        lr.copied_bytes += t.len() as u64;
                    }
                    if let Some(tl) = n.template_literal_like_data() {
                        if !in_text(tl.raw_text(), text) {
                            lr.raw_copied_bytes += tl.raw_text().len() as u64;
                        }
                    }
                }
            }
        }
        let child_top = if kind == Kind::QualifiedName { f.top } else { kind };
        let child_jsdoc = f.jsdoc || is_jsdoc_kind(kind);
        LISTS.with(|l| l.borrow_mut().clear());
        n.for_each_child(&mut |ch| {
            stack.push(Frame { node: ch, parent: Some(n), top: child_top, jsdoc: child_jsdoc });
            false
        });
        LISTS.with(|l| {
            for s in l.borrow().iter() {
                let b = bucket(s.len);
                let lk = LIST_KINDS.iter().position(|(x, _)| *x == s.kind).unwrap();
                let lr = &mut c.lists[lk][b];
                lr.count += 1;
                lr.elements += s.len as u64;
                lr.slice_bytes += 4 * s.len as u64;
                lr.struct_bytes += match s.kind {
                    ListKind::Plain => size_of::<NodeList>() as u64,
                    ListKind::Modifiers => size_of::<ModifierList>().next_multiple_of(8) as u64,
                    ListKind::LazyForced | ListKind::LazyUnforced => {
                        (size_of::<NodeList>() + size_of::<LazyNodeList>().next_multiple_of(8)) as u64
                    }
                };
                c.list_owner[k][b] += 1;
            }
        });
    }
}

/// Runs a count over `files` if `TSRS_AST_SIZING` names an output file; `skip_freed_leaves` after the pass, when the
/// check leaves' trees are freed.
pub fn run(phase: &str, files: &[P<SourceFile>], skip_freed_leaves: bool) {
    let Ok(path) = std::env::var("TSRS_AST_SIZING") else {
        return;
    };
    let start = std::time::Instant::now();
    ACTIVE.with(|a| a.set(true));
    let mut counts = [ClassCounts::new(), ClassCounts::new(), ClassCounts::new()];
    let mut seen = Seen::new();
    let mut stack: Vec<Frame> = Vec::with_capacity(1 << 16);
    let mut skipped = 0u64;
    let mut texts = Texts::default();
    for &file in files {
        let class = if file.is_check_leaf() {
            2
        } else if file.is_declaration_file() {
            1
        } else {
            0
        };
        if skip_freed_leaves && file.is_check_leaf() {
            skipped += 1;
            continue;
        }
        let c = &mut counts[class];
        let text = file.text();
        texts.file.clear();
        texts.file_names.clear();
        c.files += 1;
        c.text_bytes += text.len() as u64;
        stack.push(Frame { node: file.as_node(), parent: None, top: Kind::SourceFile, jsdoc: false });
        walk(c, &mut stack, &mut seen, text, &mut texts);
        let docs: Vec<(P<Node>, &'static [P<Node>])> = file.jsdoc_cache.borrow().iter().map(|(&h, &d)| (h, d)).collect();
        for (host, d) in docs {
            c.jsdoc_cache_entries += 1;
            c.jsdoc_cache_slice_bytes += 4 * d.len() as u64;
            for &j in d {
                stack.push(Frame { node: j, parent: Some(host), top: Kind::JSDoc, jsdoc: true });
            }
            walk(c, &mut stack, &mut seen, text, &mut texts);
        }
    }
    ACTIVE.with(|a| a.set(false));

    let mut out = String::new();
    if phase == "pre" {
        for (name, alloc, data, align, rare) in layout_rows() {
            let _ = writeln!(out, "{phase}\t-\tlayout\t{name}\t{alloc}\t{data}\t{align}\t{rare}");
        }
    }
    let _ = writeln!(out, "{phase}\t-\tmisc\tskipped_freed_leaf_files\t{skipped}");
    let bytes = |set: &FxHashSet<&'static str>| set.iter().map(|t| t.len() as u64).sum::<u64>();
    let _ = writeln!(out, "{phase}\t-\tunique\tglobal_all\t{}\t{}", texts.global.len(), bytes(&texts.global));
    let _ = writeln!(out, "{phase}\t-\tunique\tglobal_names\t{}\t{}", texts.global_names.len(), bytes(&texts.global_names));
    for (ci, c) in counts.iter().enumerate() {
        let cl = CLASSES[ci];
        let _ = writeln!(out, "{phase}\t{cl}\tfile\tfiles\t{}\t{}", c.files, c.text_bytes);
        let _ = writeln!(out, "{phase}\t{cl}\tmisc\tdups\t{}", c.dups);
        let _ = writeln!(out, "{phase}\t{cl}\tunique\tper_file_all\t{}\t{}", c.file_unique[0], c.file_unique[1]);
        let _ = writeln!(out, "{phase}\t{cl}\tunique\tper_file_names\t{}\t{}", c.file_unique_names[0], c.file_unique_names[1]);
        for (t, r) in c.tags.iter().enumerate() {
            if r[0] > 0 {
                let _ = writeln!(out, "{phase}\t{cl}\ttag\t{}\t{}\t{}\t{}", tag_name(t), r[0], r[1], r[2]);
            }
        }
        let _ = writeln!(out, "{phase}\t{cl}\tmisc\tjsdoc_cache\t{}\t{}", c.jsdoc_cache_entries, c.jsdoc_cache_slice_bytes);
        for (k, r) in c.kinds.iter().enumerate() {
            if r.count > 0 {
                let name = format!("{:?}", Kind::from_i16(k as i16));
                let _ = writeln!(out, "{phase}\t{cl}\tkind\t{name}\t{}\t{}\t{}\t{}\t{}", r.count, r.bytes, r.with_id, r.with_rare, r.in_jsdoc);
            }
        }
        for (ri, r) in c.idents.iter().enumerate() {
            if r.count > 0 {
                let _ = writeln!(
                    out,
                    "{phase}\t{cl}\tident\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    ROLE_NAMES[ri],
                    r.count,
                    r.bytes,
                    r.compact,
                    r.stored_borrow,
                    r.stored_copy,
                    r.copy_text_bytes,
                    r.text_bytes,
                    r.with_flow,
                    r.with_id
                );
            }
        }
        for (ri, per) in c.ident_parent.iter().enumerate() {
            for (k, &v) in per.iter().enumerate() {
                if v > 0 {
                    let name = format!("{:?}", Kind::from_i16(k as i16));
                    let _ = writeln!(out, "{phase}\t{cl}\tident_parent\t{}\t{name}\t{v}", ROLE_NAMES[ri]);
                }
            }
        }
        for (k, r) in c.literals.iter().enumerate() {
            if r.count > 0 {
                let name = format!("{:?}", Kind::from_i16(k as i16));
                let _ = writeln!(
                    out,
                    "{phase}\t{cl}\tliteral\t{name}\t{}\t{}\t{}\t{}\t{}\t{}",
                    r.count, r.bytes, r.text_bytes, r.copied, r.copied_bytes, r.raw_copied_bytes
                );
            }
        }
        for (lk, rows) in c.lists.iter().enumerate() {
            for (b, r) in rows.iter().enumerate() {
                if r.count > 0 {
                    let _ = writeln!(
                        out,
                        "{phase}\t{cl}\tlist\t{}\t{}\t{}\t{}\t{}\t{}",
                        LIST_KINDS[lk].1, BUCKETS[b], r.count, r.struct_bytes, r.slice_bytes, r.elements
                    );
                }
            }
        }
        for (k, per) in c.list_owner.iter().enumerate() {
            for (b, &v) in per.iter().enumerate() {
                if v > 0 {
                    let name = format!("{:?}", Kind::from_i16(k as i16));
                    let _ = writeln!(out, "{phase}\t{cl}\tlist_owner\t{name}\t{}\t{v}", BUCKETS[b]);
                }
            }
        }
    }
    let _ = writeln!(out, "{phase}\t-\tmisc\twalk_ms\t{}", start.elapsed().as_millis());
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).expect("TSRS_AST_SIZING file");
    f.write_all(out.as_bytes()).expect("TSRS_AST_SIZING write");
}
