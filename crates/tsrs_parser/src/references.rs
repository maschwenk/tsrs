use tsrs_ast::{self as ast, ModifierFlags, Node, NodeFlags, SourceFile};
use tsrs_core::{self as core, tspath, Tristate, P};

pub(crate) fn collect_external_module_references(file: P<SourceFile>) {
    for &node in file.statements().nodes() {
        collect_module_references(file, node, false /*inAmbientModule*/);
    }

    if file.as_node().flags().intersects(NodeFlags::PossiblyContainsDynamicImport) || ast::is_in_js_file(Some(file.as_node())) {
        ast::for_each_dynamic_import_or_require_call(
            file,
            true, /*includeTypeSpaceImports*/
            true, /*requireStringLiteralLikeArgument*/
            &mut |_node: P<Node>, module_specifier: P<Node>| {
                let mut imports = file.imports().to_vec();
                imports.push(module_specifier);
                ast::set_imports_of_source_file(file, imports);
                false
            },
        );
    }
}

pub(crate) fn collect_module_references(file: P<SourceFile>, node: P<Node>, in_ambient_module: bool) {
    if ast::is_any_import_or_re_export(node) {
        let module_name_expr = ast::get_external_module_name(node);
        // TypeScript 1.0 spec (April 2014): 12.1.6
        // An ExternalImportDeclaration in an AmbientExternalModuleDeclaration may reference other external modules
        // only through top - level external module names. Relative external module names are not permitted.
        if let Some(module_name_expr) = module_name_expr.filter(|e| ast::is_string_literal(*e)) {
            let module_name = module_name_expr.text();
            if !module_name.is_empty() && (!in_ambient_module || !tspath::is_external_module_name_relative(module_name)) {
                let mut imports = file.imports().to_vec();
                imports.push(module_name_expr);
                ast::set_imports_of_source_file(file, imports);
                // !!! removed `&& p.currentNodeModulesDepth == 0`
                if file.uses_uri_style_node_core_modules.get() != Tristate::True && !file.is_declaration_file.get() {
                    if module_name.starts_with("node:") && !core::exclusively_prefixed_node_core_modules(module_name) {
                        // Presence of `node:` prefix takes precedence over unprefixed node core modules
                        file.uses_uri_style_node_core_modules.set(Tristate::True);
                    } else if file.uses_uri_style_node_core_modules.get() == Tristate::Unknown
                        && core::unprefixed_node_core_modules(module_name)
                    {
                        // Avoid `unprefixedNodeCoreModules.has` for every import
                        file.uses_uri_style_node_core_modules.set(Tristate::False);
                    }
                }
            }
        }
        return;
    }
    if ast::is_module_declaration(node)
        && ast::is_ambient_module(node)
        && (in_ambient_module || ast::has_syntactic_modifier(node, ModifierFlags::Ambient) || file.is_declaration_file.get())
    {
        let name_text = node.as_module_declaration().name().text();
        // Ambient module declarations can be interpreted as augmentations for some existing external modules.
        // This will happen in two cases:
        // - if current file is external module then module augmentation is a ambient module declaration defined in the top level scope
        // - if current file is not external module then module augmentation is an ambient module declaration with non-relative module name
        //   immediately nested in top level ambient module declaration .
        if ast::is_external_module(file) || (in_ambient_module && !tspath::is_external_module_name_relative(name_text)) {
            file.module_augmentations.borrow_mut().push(node.as_module_declaration().name());
        } else if !in_ambient_module {
            file.ambient_module_names.borrow_mut().push(name_text);
            // An AmbientExternalModuleDeclaration declares an external module.
            // This type of declaration is permitted only in the global module.
            // The StringLiteral must specify a top - level external module name.
            // Relative external module names are not permitted
            // NOTE: body of ambient module is always a module block, if it exists
            if let Some(body) = node.body() {
                for &statement in body.statements() {
                    collect_module_references(file, statement, true /*inAmbientModule*/);
                }
            }
        }
    }
}
