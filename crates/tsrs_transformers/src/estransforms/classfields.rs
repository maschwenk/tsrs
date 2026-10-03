use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of classfields.go is not.

// classfields.go:88
pub struct classFieldsTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
    pub resolver: ReferenceResolverRef,

    // Computed configuration flags
    pub should_transform_initializers_using_set: Cell<bool>,
    pub should_transform_initializers_using_define: Cell<bool>,
    pub should_transform_initializers: Cell<bool>,
    pub should_transform_private_elements_or_class_static_blocks: Cell<bool>,
    pub should_transform_auto_accessors: Cell<bool>,
    pub should_transform_this_in_static_initializers: Cell<bool>,
    pub should_transform_super_in_static_initializers: Cell<bool>,
    pub should_transform_private_static_elements_in_file: Cell<bool>,
    pub legacy_decorators: bool,

    pub current_class_container: Cell<Option<P<Node>>>,
    pub class_aliases: RefCell<FxHashMap<P<Node>, P<Node>>>,
    pub parent_node: Cell<Option<P<Node>>>,
    pub current_node: Cell<Option<P<Node>>>,
}

// classfields.go:140
pub fn new_class_fields_transformer(opts: &TransformOptions) -> Option<P<Transformer>> {
    let language_version = opts.compiler_options.get_emit_script_target();
    let use_define_for_class_fields = opts.compiler_options.get_use_define_for_class_fields();

    if language_version >= ScriptTarget::ESNext && use_define_for_class_fields {
        return None;
    }

    let tx = P::new(classFieldsTransformer {
        base: Transformer::default(),
        compiler_options: opts.compiler_options,
        resolver: opts.resolver,
        should_transform_initializers_using_set: Cell::new(false),
        should_transform_initializers_using_define: Cell::new(false),
        should_transform_initializers: Cell::new(false),
        should_transform_private_elements_or_class_static_blocks: Cell::new(false),
        should_transform_auto_accessors: Cell::new(false),
        should_transform_this_in_static_initializers: Cell::new(false),
        should_transform_super_in_static_initializers: Cell::new(false),
        should_transform_private_static_elements_in_file: Cell::new(false),
        legacy_decorators: opts.compiler_options.experimental_decorators.is_true(),
        current_class_container: Cell::new(None),
        class_aliases: RefCell::new(FxHashMap::default()),
        parent_node: Cell::new(None),
        current_node: Cell::new(None),
    });

    tx.should_transform_initializers_using_set.set(!use_define_for_class_fields);
    tx.should_transform_initializers_using_define.set(use_define_for_class_fields && language_version < ScriptTarget::ES2022);
    tx.should_transform_initializers.set(tx.should_transform_initializers_using_set.get() || tx.should_transform_initializers_using_define.get());
    tx.should_transform_private_elements_or_class_static_blocks.set(language_version < ScriptTarget::ES2022);
    tx.should_transform_auto_accessors.set(language_version < ScriptTarget::ESNext);
    tx.should_transform_this_in_static_initializers.set(language_version < ScriptTarget::ES2022);
    tx.should_transform_super_in_static_initializers.set(tx.should_transform_this_in_static_initializers.get());

    let result = tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opts.context));
    // TODO(emit/classfields): the secondary visitors (modifierVisitor, discardedValueVisitor, ...).
    Some(result)
}

impl classFieldsTransformer {
    // classfields.go:248
    fn push_node(&self, node: P<Node>) -> Option<P<Node>> {
        let grandparent_node = self.parent_node.get();
        self.parent_node.set(self.current_node.get());
        self.current_node.set(Some(node));
        grandparent_node
    }

    // classfields.go:255
    fn pop_node(&self, grandparent_node: Option<P<Node>>) {
        self.current_node.set(self.parent_node.get());
        self.parent_node.set(grandparent_node);
    }

    // classfields.go:276
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        let grandparent_node = self.push_node(node);
        let result = self.visit_worker(node);
        self.pop_node(grandparent_node);
        result
    }

    fn visit_worker(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsClassFields | SubtreeFacts::ContainsLexicalThisOrSuper) {
            if self.current_class_container.get().is_some() && !self.class_aliases.borrow().is_empty() {
                unimplemented!("emit: classfields.go not ported (visitForSubstitution)");
            }
            return Some(node);
        }
        unimplemented!("emit: classfields.go not ported")
    }
}
