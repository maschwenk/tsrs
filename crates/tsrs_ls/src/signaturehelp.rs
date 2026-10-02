// PARTIAL port of signaturehelp.go (signature help is phase 3): the invocation types and the argument-list
// analysis that completions call (`getImmediatelyContainingArgumentInfo` and what it needs), in Go order.

use tsrs_ast::{self as ast, Kind, Node, NodeList, SourceFile, Symbol};
use tsrs_checker::{self as checker, Checker, ElementFlags, Signature};
use tsrs_core::{TextRange, P};
use tsrs_scanner as scanner;

use crate::utilities::{
    find_containing_list, get_possible_type_arguments_info, is_inside_template_literal, is_no_substitution_template_literal, is_tagged_template_expression,
    is_template_head, is_template_tail,
};

// signaturehelp.go:29
#[derive(Clone, Copy)]
pub(crate) struct callInvocation {
    pub(crate) node: P<Node>,
}

// signaturehelp.go:33
#[derive(Clone, Copy)]
pub(crate) struct typeArgsInvocation {
    pub(crate) called: P<Node>,
}

// signaturehelp.go:37
#[derive(Clone, Copy)]
pub(crate) struct contextualInvocation {
    pub(crate) signature: P<Signature>,
    pub(crate) node: P<Node>, // Just for enclosingDeclaration for printing types
    pub(crate) symbol: P<Symbol>,
}

// signaturehelp.go:43
#[derive(Clone, Copy, Default)]
pub(crate) struct invocation {
    pub(crate) call_invocation: Option<callInvocation>,
    pub(crate) type_args_invocation: Option<typeArgsInvocation>,
    pub(crate) contextual_invocation: Option<contextualInvocation>,
}

// signaturehelp.go:912
#[derive(Clone, Copy)]
pub(crate) struct argumentListInfo {
    pub(crate) is_type_parameter_list: bool,
    pub(crate) invocation: invocation,
    pub(crate) arguments_span: TextRange,
    pub(crate) argument_index: i32,
    // argumentCount is the *apparent* number of arguments.
    pub(crate) argument_count: i32,
}

// Returns relevant information for the argument list and the current argument if we are
// in the argument of an invocation; returns undefined otherwise.
// signaturehelp.go:923
pub(crate) fn get_immediately_containing_argument_info(node: P<Node>, position: i32, source_file: P<SourceFile>, c: &mut Checker) -> Option<argumentListInfo> {
    let parent = node.parent().unwrap();
    if ast::is_call_or_new_expression(parent) {
        // There are 3 cases to handle:
        //   1. The token introduces a list, and should begin a signature help session
        //   2. The token is either not associated with a list, or ends a list, so the session should end
        //   3. The token is buried inside a list, and should give signature help
        //
        // The following are examples of each:
        //
        //    Case 1:
        //          foo<#T, U>(#a, b)    -> The token introduces a list, and should begin a signature help session
        //    Case 2:
        //          fo#o<T, U>#(a, b)#   -> The token is either not associated with a list, or ends a list, so the session should end
        //    Case 3:
        //          foo<T#, U#>(a#, #b#) -> The token is buried inside a list, and should give signature help
        // Find out if 'node' is an argument, a type argument, or neither
        let info = get_argument_or_parameter_list_info(node, source_file, c)?;
        let list = info.list;
        let argument_index = info.argument_index;
        let argument_count = info.argument_count;
        let arguments_span = info.arguments_span;
        let mut is_type_parameter_list = false;
        let parent_type_argument_list = parent.type_argument_list();
        if let Some(parent_type_argument_list) = parent_type_argument_list {
            // Go dereferences `list` here; it is nil only for `foo(` / `foo<` with a missing list.
            if parent_type_argument_list.pos() == list.unwrap().pos() {
                is_type_parameter_list = true;
            }
        }
        return Some(argumentListInfo {
            is_type_parameter_list,
            invocation: invocation { call_invocation: Some(callInvocation { node: parent }), ..Default::default() },
            arguments_span,
            argument_index,
            argument_count,
        });
    } else if is_no_substitution_template_literal(node) && is_tagged_template_expression(parent) {
        // Check if we're actually inside the template;
        // otherwise we'll fall out and return undefined.
        if is_inside_template_literal(node, position, source_file) {
            return Some(get_argument_list_info_for_template(parent, 0, source_file));
        }
        return None;
    } else if is_template_head(node) && parent.parent().unwrap().kind() == Kind::TaggedTemplateExpression {
        let template_expression = parent;
        let tag_expression = template_expression.parent().unwrap();

        let mut argument_index = 1;
        if is_inside_template_literal(node, position, source_file) {
            argument_index = 0;
        }
        return Some(get_argument_list_info_for_template(tag_expression, argument_index, source_file));
    } else if ast::is_template_span(parent) && is_tagged_template_expression(parent.parent().unwrap().parent().unwrap()) {
        let template_span = parent;
        let tag_expression = parent.parent().unwrap().parent().unwrap();

        // If we're just after a template tail, don't show signature help.
        if is_template_tail(node) && !is_inside_template_literal(node, position, source_file) {
            return None;
        }

        let span_index = ast::index_of_node(template_span.parent().unwrap().as_template_expression().template_spans.nodes(), template_span);
        let argument_index = get_argument_index_for_template_piece(span_index, node, position, source_file);

        return Some(get_argument_list_info_for_template(tag_expression, argument_index, source_file));
    } else if ast::is_jsx_opening_like_element(parent) {
        // Provide a signature help for JSX opening element or JSX self-closing element.
        // This is not guarantee that JSX tag-name is resolved into stateless function component. (that is done in "getSignatureHelpItems")
        // i.e
        //      export function MainButton(props: ButtonProps, context: any): JSX.Element { ... }
        //      <MainButton /*signatureHelp*/
        let attribute_span_start = parent.attributes().unwrap().loc().pos();
        let attribute_span_end = scanner::skip_trivia(source_file.text(), parent.attributes().unwrap().end());
        return Some(argumentListInfo {
            is_type_parameter_list: false,
            invocation: invocation { call_invocation: Some(callInvocation { node: parent }), ..Default::default() },
            arguments_span: TextRange::new(attribute_span_start, attribute_span_end - attribute_span_start),
            argument_index: 0,
            argument_count: 1,
        });
    } else {
        let type_arg_info = get_possible_type_arguments_info(Some(node), source_file);
        if let Some(type_arg_info) = type_arg_info {
            let called = type_arg_info.called;
            let n_type_arguments = type_arg_info.n_type_arguments as i32;
            let invoc = typeArgsInvocation { called };
            let argument_range = TextRange::new(called.loc().pos(), node.end());
            return Some(argumentListInfo {
                is_type_parameter_list: true,
                invocation: invocation { type_args_invocation: Some(invoc), ..Default::default() },
                arguments_span: argument_range,
                argument_index: n_type_arguments,
                argument_count: n_type_arguments + 1,
            });
        }
    }
    None
}

// spanIndex is either the index for a given template span.
// This does not give appropriate results for a NoSubstitutionTemplateLiteral
// signaturehelp.go:1029
fn get_argument_index_for_template_piece(span_index: i32, node: P<Node>, position: i32, source_file: P<SourceFile>) -> i32 {
    // Because the TemplateStringsArray is the first argument, we have to offset each substitution expression by 1.
    // There are three cases we can encounter:
    //      1. We are precisely in the template literal (argIndex = 0).
    //      2. We are in or to the right of the substitution expression (argIndex = spanIndex + 1).
    //      3. We are directly to the right of the template literal, but because we look for the token on the left,
    //          not enough to put us in the substitution expression; we should consider ourselves part of
    //          the *next* span's expression by offsetting the index (argIndex = (spanIndex + 1) + 1).
    //
    // Example: f  `# abcd $#{#  1 + 1#  }# efghi ${ #"#hello"#  }  #  `
    //              ^       ^ ^       ^   ^          ^ ^      ^     ^
    // Case:        1       1 3       2   1          3 2      2     1
    assert!(position >= node.loc().pos(), "Assumed 'position' could not occur before node.");
    if ast::is_template_literal_token(node) {
        if is_inside_template_literal(node, position, source_file) {
            return 0;
        }
        return span_index + 2;
    }
    span_index + 1
}

// signaturehelp.go:1074
fn get_spread_element_count(node: P<Node>, c: &mut Checker) -> i32 {
    let spread_type = c.get_type_at_location(node.expression().unwrap());
    if checker::is_tuple_type_exported(spread_type) {
        let tuple_type = spread_type.target().unwrap().as_tuple_type();
        let element_flags = tuple_type.element_flags();
        let fixed_length = tuple_type.fixed_length();
        if fixed_length == 0 {
            return 0;
        }

        let first_optional_index = element_flags.iter().position(|f| !f.intersects(ElementFlags::Required));
        let Some(first_optional_index) = first_optional_index else {
            return fixed_length;
        };
        return first_optional_index as i32;
    }
    0
}

// signaturehelp.go:1098
fn get_argument_index(node: P<Node>, arguments: Option<P<NodeList>>, source_file: P<SourceFile>, c: &mut Checker) -> i32 {
    get_argument_index_or_count(&get_token_from_node_list(arguments, node.parent(), source_file), Some(node), c)
}

// signaturehelp.go:1102
fn get_argument_count(node: P<Node>, arguments: Option<P<NodeList>>, source_file: P<SourceFile>, c: &mut Checker) -> i32 {
    get_argument_index_or_count(&get_token_from_node_list(arguments, node.parent(), source_file), None, c)
}

// signaturehelp.go:1106
fn get_argument_index_or_count(arguments: &[P<Node>], node: Option<P<Node>>, c: &mut Checker) -> i32 {
    let mut argument_index = 0;
    let mut skip_comma = false;
    for &arg in arguments {
        if node.is_some() && Some(arg) == node {
            if !skip_comma && arg.kind() == Kind::CommaToken {
                argument_index += 1;
            }
            return argument_index;
        }
        if ast::is_spread_element(arg) {
            argument_index += get_spread_element_count(arg, c);
            skip_comma = true;
            continue;
        }
        if arg.kind() != Kind::CommaToken {
            argument_index += 1;
            skip_comma = true;
            continue;
        }
        if skip_comma {
            skip_comma = false;
            continue;
        }
        argument_index += 1;
    }
    if node.is_some() {
        return argument_index;
    }
    // The argument count for a list is normally the number of non-comma children it has.
    // For example, if you have "Foo(a,b)" then there will be three children of the arg
    // list 'a' '<comma>' 'b'. So, in this case the arg count will be 2. However, there
    // is a small subtlety. If you have "Foo(a,)", then the child list will just have
    // 'a' '<comma>'. So, in the case where the last child is a comma, we increase the
    // arg count by one to compensate.
    let mut argument_count = argument_index;
    if !arguments.is_empty() && arguments[arguments.len() - 1].kind() == Kind::CommaToken {
        argument_count = argument_index + 1;
    }
    argument_count
}

// signaturehelp.go:1148
struct argumentOrParameterListInfo {
    list: Option<P<NodeList>>,
    argument_index: i32,
    argument_count: i32,
    arguments_span: TextRange,
}

// signaturehelp.go:1155
fn get_argument_or_parameter_list_info(node: P<Node>, source_file: P<SourceFile>, c: &mut Checker) -> Option<argumentOrParameterListInfo> {
    let info = get_argument_or_parameter_list_and_index(node, source_file, c)?;
    let list = info.list;
    let argument_index = info.argument_index;
    let argument_count = get_argument_count(node, list, source_file, c);
    let arguments_span = get_applicable_span_for_arguments(list, Some(node), source_file);
    Some(argumentOrParameterListInfo { list, argument_index, argument_count, arguments_span })
}

// signaturehelp.go:1172
fn get_applicable_span_for_arguments(argument_list: Option<P<NodeList>>, node: Option<P<Node>>, source_file: P<SourceFile>) -> TextRange {
    // We use full start and skip trivia on the end because we want to include trivia on
    // both sides. For example,
    //
    //    foo(   /*comment */     a, b, c      /*comment*/     )
    //        |                                               |
    //
    // The applicable span is from the first bar to the second bar (inclusive,
    // but not including parentheses).
    if argument_list.is_none() {
        if let Some(node) = node {
            // If the user has just opened a list, and there are no arguments.
            // For example, foo(    )
            //                  |  |
            // The span should include positions inside the parentheses.
            let span_start = node.end();
            let mut span_end = scanner::skip_trivia(source_file.text(), node.end());
            span_end = ensure_minimum_span_size(span_start, span_end);
            return TextRange::new(span_start, span_end);
        }
    }
    let argument_list = argument_list.unwrap();
    let applicable_span_start = argument_list.pos();
    let mut applicable_span_end = scanner::skip_trivia(source_file.text(), argument_list.end());

    // If the argument list is empty (Pos == End), extend the span to include at least
    // one position. This handles foo(|) where the cursor is right after the opening paren.
    applicable_span_end = ensure_minimum_span_size(applicable_span_start, applicable_span_end);

    TextRange::new(applicable_span_start, applicable_span_end)
}

// ensureMinimumSpanSize ensures that a span includes at least one position.
// TextRange.Contains uses a half-open interval, so an empty span would not contain
// the cursor immediately after typing an opening paren in a call like foo(bar(|)).
// signaturehelp.go:1204
fn ensure_minimum_span_size(start: i32, end: i32) -> i32 {
    if end <= start {
        return start + 1;
    }
    end
}

// signaturehelp.go:1211
struct argumentOrParameterListAndIndex {
    list: Option<P<NodeList>>,
    argument_index: i32,
}

// signaturehelp.go:1216
fn get_argument_or_parameter_list_and_index(node: P<Node>, source_file: P<SourceFile>, c: &mut Checker) -> Option<argumentOrParameterListAndIndex> {
    if node.kind() == Kind::LessThanToken || node.kind() == Kind::OpenParenToken {
        // Find the list that starts right *after* the < or ( token.
        // If the user has just opened a list, consider this item 0.
        let list = get_child_list_that_starts_with_opener_token(node.parent().unwrap(), node);
        Some(argumentOrParameterListAndIndex { list, argument_index: 0 })
    } else {
        // findListItemInfo can return undefined if we are not in parent's argument list
        // or type argument list. This includes cases where the cursor is:
        //   - To the right of the closing parenthesis, non-substitution template, or template tail.
        //   - Between the type arguments and the arguments (greater than token)
        //   - On the target of the call (parent.func)
        //   - On the 'new' keyword in a 'new' expression
        let list = find_containing_list(node, source_file)?;
        Some(argumentOrParameterListAndIndex {
            list: Some(list),
            // Find the index of the argument that contains the node.
            argument_index: get_argument_index(node, Some(list), source_file, c),
        })
    }
}

// signaturehelp.go:1244
fn get_child_list_that_starts_with_opener_token(parent: P<Node>, opener_token: P<Node>) -> Option<P<NodeList>> {
    if ast::is_call_expression(parent) {
        if opener_token.kind() == Kind::LessThanToken {
            return parent.type_argument_list();
        }
        return parent.argument_list();
    } else if ast::is_new_expression(parent) {
        if opener_token.kind() == Kind::LessThanToken {
            return parent.type_argument_list();
        }
        return parent.argument_list();
    }
    None
}

// signaturehelp.go:1375
fn get_token_from_node_list(node_list: Option<P<NodeList>>, node_list_parent: Option<P<Node>>, source_file: P<SourceFile>) -> Vec<P<Node>> {
    let (Some(node_list), Some(node_list_parent)) = (node_list, node_list_parent) else {
        return Vec::new();
    };
    let mut left = node_list.pos();
    let mut node_list_index = 0;
    let mut tokens = Vec::new();
    let nodes = node_list.nodes();
    while left < node_list.end() {
        if nodes.len() > node_list_index && left == nodes[node_list_index].pos() {
            tokens.push(nodes[node_list_index]);
            left = nodes[node_list_index].end();
            node_list_index += 1;
        } else {
            let scanner = scanner::get_scanner_for_source_file(source_file, left);
            let token = scanner.token();
            let token_full_start = scanner.token_full_start();
            let token_end = scanner.token_end();
            tokens.push(source_file.get_or_create_token(token, token_full_start, token_end, node_list_parent, scanner.token_flags()));
            left = token_end;
        }
    }
    tokens
}

// signaturehelp.go:1399
fn get_argument_list_info_for_template(tag_expression: P<Node>, argument_index: i32, source_file: P<SourceFile>) -> argumentListInfo {
    // argumentCount is either 1 or (numSpans + 1) to account for the template strings array argument.
    let template = tag_expression.as_tagged_template_expression().template;
    let mut argument_count = 1;
    if !is_no_substitution_template_literal(template) {
        argument_count = template.as_template_expression().template_spans.nodes().len() as i32 + 1;
    }
    if argument_index != 0 {
        assert!(argument_index < argument_count);
    }
    argumentListInfo {
        is_type_parameter_list: false,
        invocation: invocation { call_invocation: Some(callInvocation { node: tag_expression }), ..Default::default() },
        argument_index,
        argument_count,
        arguments_span: get_applicable_range_for_tagged_template(tag_expression, source_file),
    }
}

// signaturehelp.go:1417
fn get_applicable_range_for_tagged_template(tagged_template: P<Node>, source_file: P<SourceFile>) -> TextRange {
    let template = tagged_template.as_tagged_template_expression().template;
    let applicable_span_start = scanner::get_token_pos_of_node(template, source_file, false);
    let mut applicable_span_end = template.end();

    // We need to adjust the end position for the case where the template does not have a tail.
    // Otherwise, we will not show signature help past the expression.
    // For example,
    //
    //      ` ${ 1 + 1 foo(10)
    //       |       |
    // This is because a Missing node has no width. However, what we actually want is to include trivia
    // leading up to the next token in case the user is about to type in a TemplateMiddle or TemplateTail.
    if template.kind() == Kind::TemplateExpression {
        let template_spans = template.as_template_expression().template_spans;
        let last_span = template_spans.nodes()[template_spans.nodes().len() - 1];
        if last_span.as_template_span().literal.end() - last_span.as_template_span().literal.pos() == 0 {
            applicable_span_end = scanner::skip_trivia(source_file.text(), applicable_span_end);
        }
    }

    TextRange::new(applicable_span_start, applicable_span_end - applicable_span_start)
}
