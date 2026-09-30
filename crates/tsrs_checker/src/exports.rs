use crate::*;
use tsrs_ast::*;
use tsrs_core::*;
use tsrs_ast as ast;
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;
use rustc_hash::FxHashMap;
use std::fmt::Display;

impl Checker {
    // exports.go:8
    pub fn get_string_type(&mut self) -> P<Type> {
        self.string_type
    }

    // exports.go:12
    pub fn get_number_type(&mut self) -> P<Type> {
        self.number_type
    }

    // exports.go:16
    pub fn get_boolean_type(&mut self) -> P<Type> {
        self.boolean_type
    }

    // exports.go:20
    pub fn get_void_type(&mut self) -> P<Type> {
        self.void_type
    }

    // exports.go:24
    pub fn get_undefined_type(&mut self) -> P<Type> {
        self.undefined_type
    }

    // exports.go:28
    pub fn get_null_type(&mut self) -> P<Type> {
        self.null_type
    }

    // exports.go:32
    pub fn get_any_type(&mut self) -> P<Type> {
        self.any_type
    }

    // exports.go:36
    pub fn get_error_type(&mut self) -> P<Type> {
        self.error_type
    }

    // exports.go:40
    pub fn get_never_type(&mut self) -> P<Type> {
        self.never_type
    }

    // exports.go:44
    pub fn get_unknown_type(&mut self) -> P<Type> {
        self.unknown_type
    }

    // exports.go:48
    pub fn get_big_int_type(&mut self) -> P<Type> {
        self.bigint_type
    }

    // exports.go:52
    pub fn get_es_symbol_type(&mut self) -> P<Type> {
        self.es_symbol_type
    }

    // exports.go:56
    pub fn get_non_primitive_type(&mut self) -> P<Type> {
        self.non_primitive_type
    }

    // exports.go:64
    pub fn get_unknown_symbol(&mut self) -> P<Symbol> {
        self.unknown_symbol
    }

    // exports.go:68
    pub fn get_undefined_symbol(&mut self) -> P<Symbol> {
        self.undefined_symbol
    }

    // exports.go:72
    pub fn get_arguments_symbol(&mut self) -> P<Symbol> {
        self.arguments_symbol
    }

    // exports.go:76
    pub fn get_unknown_signature(&mut self) -> P<Signature> {
        self.unknown_signature
    }

    // exports.go:84
    pub fn get_name_type_of_symbol(&mut self, symbol: P<Symbol>) -> Option<P<Type>> {
        if let Some(links) = self.value_symbol_links.try_get(symbol) {
            return links.name_type.get();
        }
        None
    }

    // exports.go:99
    pub fn get_global_symbol_exported(&mut self, name: &str, meaning: SymbolFlags, diagnostic: &'static Message) -> Option<P<Symbol>> {
        self.get_global_symbol(name, meaning, Some(diagnostic))
    }

    // exports.go:107
    pub fn try_find_ambient_module_exported(&mut self, module_name: &str) -> Option<P<Symbol>> {
        self.try_find_ambient_module(module_name, true /* withAugmentations */)
    }

    // exports.go:115
    pub fn get_target_symbol_exported(&mut self, symbol: P<Symbol>) -> Option<P<Symbol>> {
        self.get_target_symbol(symbol)
    }

    // exports.go:123
    pub fn resolve_external_module_name_exported(&mut self, module_specifier: P<Node>, import_attributes_type: P<Type>) -> Option<P<Symbol>> {
        self.resolve_external_module_name(module_specifier, module_specifier, true /*ignoreErrors*/, Some(import_attributes_type))
    }

    // exports.go:127
    pub fn resolve_external_module_symbol_exported(&mut self, module_symbol: P<Symbol>) -> P<Symbol> {
        self.resolve_external_module_symbol(module_symbol, false /*dontResolveAlias*/)
    }

    // exports.go:171
    pub fn was_canceled(&mut self) -> bool {
        self.was_canceled
    }

    // exports.go:195
    pub fn get_true_type_of_conditional_type(&mut self, t: P<Type>) -> P<Type> {
        self.get_true_type_from_conditional_type(t)
    }

    // exports.go:199
    pub fn get_false_type_of_conditional_type(&mut self, t: P<Type>) -> P<Type> {
        self.get_false_type_from_conditional_type(t)
    }

    // exports.go:203
    pub fn get_default_from_type_parameter_exported(&mut self, type_parameter: P<Type>) -> Option<P<Type>> {
        self.get_default_from_type_parameter(type_parameter)
    }
}

// exports.go:223
pub fn is_tuple_type_target(t: P<Type>) -> bool {
    is_tuple_type(t) && t.target() == Some(t)
}

impl Checker {
    // exports.go:251
    pub fn type_predicate_to_string_exported(&mut self, t: P<TypePredicate>) -> String {
        self.type_predicate_to_string(t)
    }

    // exports.go:255
    pub fn get_expanded_parameters_exported(&mut self, signature: P<Signature>, skip_union_expanding: bool) -> Vec<Vec<P<Symbol>>> {
        // getExpandedParameters lives in nodebuilderimpl.go (node builder, not ported yet); only the
        // language service calls this wrapper.
        let _ = (signature, skip_union_expanding);
        unimplemented!("node builder")
    }

    // exports.go:259
    pub fn get_resolved_signature_exported(&mut self, node: P<Node>) -> P<Signature> {
        self.get_resolved_signature(node, None, CheckMode::Normal)
    }

    // exports.go:268
    pub fn get_contextual_type_for_argument_at_index_exported(&mut self, node: P<Node>, arg_index: i32) -> Option<P<Type>> {
        self.get_contextual_type_for_argument_at_index(node, arg_index)
    }

    // exports.go:284
    pub fn get_jsx_namespace_exported(&mut self, location: P<Node>) -> String {
        self.get_jsx_namespace(Some(location))
    }

    // exports.go:288
    pub fn get_jsx_fragment_factory(&mut self, location: P<Node>) -> String {
        let entity = self.get_jsx_fragment_factory_entity(Some(location));
        if let Some(entity) = entity {
            return ast::get_first_identifier(entity).text().to_string();
        }
        String::new()
    }

    // exports.go:296
    pub fn resolve_name_exported(&mut self, name: &str, location: P<Node>, meaning: SymbolFlags, exclude_globals: bool) -> Option<P<Symbol>> {
        self.resolve_name(Some(location), name, meaning, None, true, exclude_globals)
    }

    // exports.go:318
    pub fn get_fully_qualified_name_exported(&mut self, symbol: P<Symbol>) -> String {
        self.get_fully_qualified_name(symbol, None /*containingLocation*/)
    }

    // exports.go:326
    pub fn get_member_override_modifier_status_exported(&mut self, node: P<Node>, member: P<Node>, member_symbol: P<Symbol>) -> MemberOverrideStatus {
        self.get_member_override_modifier_status(node, member, Some(member_symbol))
    }

    // exports.go:330
    pub fn get_rest_type_of_signature_exported(&mut self, sig: P<Signature>) -> P<Type> {
        self.get_rest_type_of_signature(sig)
    }

    // exports.go:370
    pub fn get_union_type_ex_exported(&mut self, types: &[P<Type>], union_reduction: UnionReduction) -> P<Type> {
        self.get_union_type_ex(types, union_reduction, None, None)
    }

    // exports.go:374
    pub fn requires_adding_implicit_undefined(&mut self, node: P<Node>) -> bool {
        let enclosing_declaration = ast::find_ancestor(node, ast::is_declaration);
        let enclosing_declaration = match enclosing_declaration {
            Some(d) => d,
            None => ast::get_source_file_of_node(node).unwrap().as_node(),
        };
        let symbol = node.symbol();
        if symbol.is_none() {
            return false;
        }
        // EmitResolver.RequiresAddingImplicitUndefined (emitresolver.go:568), the exported locking wrapper.
        if !ast::is_parse_tree_node(node) {
            return false;
        }
        let r = self.get_emit_resolver();
        r.requires_adding_implicit_undefined(self, node, symbol, enclosing_declaration)
    }

    // exports.go:394
    pub fn compare_symbols_exported(&mut self, s1: P<Symbol>, s2: P<Symbol>) -> i32 {
        self.compare_symbols(Some(s1), Some(s2))
    }
}

// exports.go:398
pub fn is_distributed_type_parameter(t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::TypeParameter) && t.as_type_parameter().is_distributed.get()
}
