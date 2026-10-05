use crate::*;
use tsrs_core::*;
use tsrs_ast as ast;
use tsrs_diagnostics as diagnostics;

impl Checker {
    // jsdoc.go:9
    pub(crate) fn check_unmatched_jsdoc_parameters(&mut self, node: P<Node>) {
        let mut jsdoc_parameters: Vec<P<Node>> = Vec::new();
        for tag in get_all_jsdoc_tags(node) {
            if tag.kind() == Kind::JSDocParameterTag {
                let name = tag.as_jsdoc_parameter_or_property_tag().name();
                if ast::is_identifier(name) && name.text().is_empty() {
                    continue;
                }
                jsdoc_parameters.push(tag);
            }
        }

        if jsdoc_parameters.is_empty() {
            return;
        }

        let is_js = ast::is_in_js_file(node);
        let mut parameters: collections::Set<String> = collections::Set::default();
        let mut excluded_parameters: collections::Set<i32> = collections::Set::default();

        for (i, &param) in node.parameters().iter().enumerate() {
            let name = param.as_parameter_declaration().name();
            if ast::is_identifier(name) {
                parameters.add(name.text().to_string());
            }
            if ast::is_binding_pattern(name) {
                excluded_parameters.add(i as i32);
            }
        }
        if self.contains_arguments_reference(node) {
            if is_js {
                let last_jsdoc_param_index = jsdoc_parameters.len() as i32 - 1;
                let last_jsdoc_param = jsdoc_parameters[last_jsdoc_param_index as usize].as_jsdoc_parameter_or_property_tag();
                if !ast::is_identifier(last_jsdoc_param.name()) {
                    return;
                }
                if excluded_parameters.has(&last_jsdoc_param_index) || parameters.has(&last_jsdoc_param.name().text().to_string()) {
                    return;
                }
                let Some(type_expression) = last_jsdoc_param.type_expression else {
                    return;
                };
                let Some(type_node) = type_expression.type_node() else {
                    return;
                };
                let t = self.get_type_from_type_node(type_node);
                if self.is_array_type(t) {
                    return;
                }
                let name = last_jsdoc_param.name();
                self.error(
                    Some(name),
                    &diagnostics::JSDoc_param_tag_has_name_0_but_there_is_no_parameter_with_that_name_It_would_match_arguments_if_it_had_an_array_type,
                    &[&name.text()],
                );
            }
        } else {
            for (index, &tag) in jsdoc_parameters.iter().enumerate() {
                let name = tag.as_jsdoc_parameter_or_property_tag().name();
                let is_name_first = tag.as_jsdoc_parameter_or_property_tag().is_name_first;

                if excluded_parameters.has(&(index as i32)) || (ast::is_identifier(name) && parameters.has(&name.text().to_string())) {
                    continue;
                }

                if ast::is_qualified_name(name) {
                    if is_js {
                        self.error(
                            Some(name),
                            &diagnostics::Qualified_name_0_is_not_allowed_without_a_leading_param_object_1,
                            &[&crate::entity_name_to_string(name), &crate::entity_name_to_string(name.as_qualified_name().left)],
                        );
                    }
                } else if !is_name_first {
                    self.error_or_suggestion(is_js, Some(name), &diagnostics::JSDoc_param_tag_has_name_0_but_there_is_no_parameter_with_that_name, &[&name.text()]);
                }
            }
        }
    }
}

// jsdoc.go:86
pub(crate) fn get_all_jsdoc_tags(node: P<Node>) -> Vec<P<Node>> {
    if !node.flags().intersects(NodeFlags::JSDoc) {
        let mut current = Some(node);
        while let Some(cur) = current {
            let jsdocs = cur.jsdoc(None);
            if !jsdocs.is_empty() {
                let last_jsdoc = jsdocs[jsdocs.len() - 1].as_jsdoc();
                if let Some(tags) = last_jsdoc.tags {
                    return tags.nodes().to_vec();
                }
            }
            current = ast::get_next_jsdoc_comment_location(cur);
        }
    }
    Vec::new()
}
