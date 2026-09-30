use std::fmt::Display;

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast as ast;
use tsrs_ast::{Kind, ModifierFlags, ModifierList, Node, NodeFlags, NodeList, OperatorPrecedence};
use tsrs_core::{LanguageVariant, TextRange, Tristate, P};
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;
use tsrs_scanner as scanner;

use crate::*;

impl Parser {
    pub(crate) fn parse_assignment_expression_or_higher(&mut self) -> P<Node> {
        self.parse_assignment_expression_or_higher_worker(true /*allowReturnTypeInArrowFunction*/)
    }

    pub(crate) fn parse_assignment_expression_or_higher_worker(&mut self, allow_return_type_in_arrow_function: bool) -> P<Node> {
        //  AssignmentExpression[in,yield]:
        //      1) ConditionalExpression[?in,?yield]
        //      2) LeftHandSideExpression = AssignmentExpression[?in,?yield]
        //      3) LeftHandSideExpression AssignmentOperator AssignmentExpression[?in,?yield]
        //      4) ArrowFunctionExpression[?in,?yield]
        //      5) AsyncArrowFunctionExpression[in,yield,await]
        //      6) [+Yield] YieldExpression[?In]
        //
        // Note: for ease of implementation we treat productions '2' and '3' as the same thing.
        // (i.e. they're both BinaryExpressions with an assignment operator in it).
        // First, do the simple check if we have a YieldExpression (production '6').
        if self.is_yield_expression() {
            return self.parse_yield_expression();
        }
        // Then, check if we have an arrow function (production '4' and '5') that starts with a parenthesized
        // parameter list or is an async arrow function.
        // AsyncArrowFunctionExpression:
        //      1) async[no LineTerminator here]AsyncArrowBindingIdentifier[?Yield][no LineTerminator here]=>AsyncConciseBody[?In]
        //      2) CoverCallExpressionAndAsyncArrowHead[?Yield, ?Await][no LineTerminator here]=>AsyncConciseBody[?In]
        // Production (1) of AsyncArrowFunctionExpression is parsed in "tryParseAsyncSimpleArrowFunctionExpression".
        // And production (2) is parsed in "tryParseParenthesizedArrowFunctionExpression".
        //
        // If we do successfully parse arrow-function, we must *not* recurse for productions 1, 2 or 3. An ArrowFunction is
        // not a LeftHandSideExpression, nor does it start a ConditionalExpression.  So we are done
        // with AssignmentExpression if we see one.
        if let Some(arrow_expression) = self.try_parse_parenthesized_arrow_function_expression(allow_return_type_in_arrow_function) {
            return arrow_expression;
        }
        if let Some(arrow_expression) = self.try_parse_async_simple_arrow_function_expression(allow_return_type_in_arrow_function) {
            return arrow_expression;
        }
        // Now try to see if we're in production '1', '2' or '3'.  A conditional expression can
        // start with a LogicalOrExpression, while the assignment productions can only start with
        // LeftHandSideExpressions.
        //
        // So, first, we try to just parse out a BinaryExpression.  If we get something that is a
        // LeftHandSide or higher, then we can try to parse out the assignment expression part.
        // Otherwise, we try to parse out the conditional expression bit.  We want to allow any
        // binary expression here, so we pass in the 'lowest' precedence here so that it matches
        // and consumes anything.
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        let expr = self.parse_binary_expression_or_higher(OperatorPrecedence::Lowest);
        // To avoid a look-ahead, we did not handle the case of an arrow function with a single un-parenthesized
        // parameter ('x => ...') above. We handle it here by checking if the parsed expression was a single
        // identifier and the current token is an arrow.
        if expr.kind == Kind::Identifier && self.token == Kind::EqualsGreaterThanToken {
            return self.parse_simple_arrow_function_expression(pos, expr, allow_return_type_in_arrow_function, jsdoc, None /*asyncModifier*/);
        }
        // Now see if we might be in cases '2' or '3'.
        // If the expression was a LHS expression, and we have an assignment operator, then
        // we're in '2' or '3'. Consume the assignment and return.
        //
        // Note: we call reScanGreaterToken so that we get an appropriately merged token
        // for cases like `> > =` becoming `>>=`
        if ast::is_left_hand_side_expression(expr) && ast::is_assignment_operator(self.re_scan_greater_than_token()) {
            let operator_token = self.parse_token_node();
            let right = self.parse_assignment_expression_or_higher_worker(allow_return_type_in_arrow_function);
            return self.make_binary_expression(expr, operator_token, right, pos);
        }
        // It wasn't an assignment or a lambda.  This is a conditional expression:
        self.parse_conditional_expression_rest(expr, pos, allow_return_type_in_arrow_function)
    }

    pub(crate) fn is_yield_expression(&mut self) -> bool {
        if self.token == Kind::YieldKeyword {
            // If we have a 'yield' keyword, and this is a context where yield expressions are
            // allowed, then definitely parse out a yield expression.
            if self.in_yield_context() {
                return true;
            }

            // We're in a context where 'yield expr' is not allowed.  However, if we can
            // definitely tell that the user was trying to parse a 'yield expr' and not
            // just a normal expr that start with a 'yield' identifier, then parse out
            // a 'yield expr'.  We can then report an error later that they are only
            // allowed in generator expressions.
            //
            // for example, if we see 'yield(foo)', then we'll have to treat that as an
            // invocation expression of something called 'yield'.  However, if we have
            // 'yield foo' then that is not legal as a normal expression, so we can
            // definitely recognize this as a yield expression.
            //
            // for now we just check if the next token is an identifier.  More heuristics
            // can be added here later as necessary.  We just need to make sure that we
            // don't accidentally consume something legal.
            return self.look_ahead(Parser::next_token_is_identifier_or_keyword_or_literal_on_same_line);
        }
        false
    }

    pub(crate) fn parse_yield_expression(&mut self) -> P<Node> {
        let pos = self.node_pos();
        // YieldExpression[In] :
        //      yield
        //      yield [no LineTerminator here] [Lexical goal InputElementRegExp]AssignmentExpression[?In, Yield]
        //      yield [no LineTerminator here] * [Lexical goal InputElementRegExp]AssignmentExpression[?In, Yield]
        self.next_token();
        let result;
        if !self.has_preceding_line_break() && (self.token == Kind::AsteriskToken || self.is_start_of_expression()) {
            let asterisk_token = self.parse_optional_token(Kind::AsteriskToken);
            let expression = self.parse_assignment_expression_or_higher();
            result = self.factory.new_yield_expression(asterisk_token, Some(expression));
        } else {
            // if the next token is not on the same line as yield.  or we don't have an '*' or
            // the start of an expression, then this is just a simple "yield" expression.
            result = self.factory.new_yield_expression(None /*asteriskToken*/, None /*expression*/);
        }
        self.finish_node(result, pos)
    }

    pub(crate) fn is_parenthesized_arrow_function_expression(&mut self) -> Tristate {
        if self.token == Kind::OpenParenToken || self.token == Kind::LessThanToken || self.token == Kind::AsyncKeyword {
            let state = self.mark();
            let result = self.next_is_parenthesized_arrow_function_expression();
            self.rewind(state);
            return result;
        }
        if self.token == Kind::EqualsGreaterThanToken {
            // ERROR RECOVERY TWEAK:
            // If we see a standalone => try to parse it as an arrow function expression as that's
            // likely what the user intended to write.
            return Tristate::True;
        }
        // Definitely not a parenthesized arrow function.
        Tristate::False
    }

    pub(crate) fn next_is_parenthesized_arrow_function_expression(&mut self) -> Tristate {
        if self.token == Kind::AsyncKeyword {
            self.next_token();
            if self.has_preceding_line_break() {
                return Tristate::False;
            }
            if self.token != Kind::OpenParenToken && self.token != Kind::LessThanToken {
                return Tristate::False;
            }
        }
        let first = self.token;
        let second = self.next_token();
        if first == Kind::OpenParenToken {
            if second == Kind::CloseParenToken {
                // Simple cases: "() =>", "(): ", and "() {".
                // This is an arrow function with no parameters.
                // The last one is not actually an arrow function,
                // but this is probably what the user intended.
                let third = self.next_token();
                match third {
                    Kind::EqualsGreaterThanToken | Kind::ColonToken | Kind::OpenBraceToken => return Tristate::True,
                    _ => {}
                }
                return Tristate::False;
            }
            // If encounter "([" or "({", this could be the start of a binding pattern.
            // Examples:
            //      ([ x ]) => { }
            //      ({ x }) => { }
            //      ([ x ])
            //      ({ x })
            if second == Kind::OpenBracketToken || second == Kind::OpenBraceToken {
                return Tristate::Unknown;
            }
            // Simple case: "(..."
            // This is an arrow function with a rest parameter.
            if second == Kind::DotDotDotToken {
                return Tristate::True;
            }
            // Check for "(xxx yyy", where xxx is a modifier and yyy is an identifier. This
            // isn't actually allowed, but we want to treat it as a lambda so we can provide
            // a good error message.
            if ast::is_modifier_kind(second) && second != Kind::AsyncKeyword && self.look_ahead(Parser::next_token_is_identifier) {
                if self.next_token() == Kind::AsKeyword {
                    // https://github.com/microsoft/TypeScript/issues/44466
                    return Tristate::False;
                }
                return Tristate::True;
            }
            // If we had "(" followed by something that's not an identifier,
            // then this definitely doesn't look like a lambda.  "this" is not
            // valid, but we want to parse it and then give a semantic error.
            if !self.is_identifier() && second != Kind::ThisKeyword {
                return Tristate::False;
            }
            match self.next_token() {
                Kind::ColonToken => {
                    // If we have something like "(a:", then we must have a
                    // type-annotated parameter in an arrow function expression.
                    return Tristate::True;
                }
                Kind::QuestionToken => {
                    self.next_token();
                    // If we have "(a?:" or "(a?," or "(a?=" or "(a?)" then it is definitely a lambda.
                    if self.token == Kind::ColonToken || self.token == Kind::CommaToken || self.token == Kind::EqualsToken || self.token == Kind::CloseParenToken {
                        return Tristate::True;
                    }
                    // Otherwise it is definitely not a lambda.
                    return Tristate::False;
                }
                Kind::CommaToken | Kind::EqualsToken | Kind::CloseParenToken => {
                    // If we have "(a," or "(a=" or "(a)" this *could* be an arrow function
                    return Tristate::Unknown;
                }
                _ => {}
            }
            // It is definitely not an arrow function
            Tristate::False
        } else {
            assert!(first == Kind::LessThanToken);
            // If we have "<" not followed by an identifier,
            // then this definitely is not an arrow function.
            if !self.is_identifier() && self.token != Kind::ConstKeyword {
                return Tristate::False;
            }
            // JSX overrides
            if self.language_variant == LanguageVariant::JSX {
                let is_arrow_function_in_jsx = self.look_ahead(|p| {
                    p.parse_optional(Kind::ConstKeyword);
                    let third = p.next_token();
                    if third == Kind::ExtendsKeyword {
                        let fourth = p.next_token();
                        match fourth {
                            Kind::EqualsToken | Kind::GreaterThanToken | Kind::SlashToken => return false,
                            _ => {}
                        }
                        return true;
                    } else if third == Kind::CommaToken || third == Kind::EqualsToken {
                        return true;
                    }
                    false
                });
                if is_arrow_function_in_jsx {
                    return Tristate::True;
                }
                return Tristate::False;
            }
            // This *could* be a parenthesized arrow function.
            Tristate::Unknown
        }
    }

    pub(crate) fn try_parse_parenthesized_arrow_function_expression(&mut self, allow_return_type_in_arrow_function: bool) -> Option<P<Node>> {
        let tristate = self.is_parenthesized_arrow_function_expression();
        if tristate == Tristate::False {
            // It's definitely not a parenthesized arrow function expression.
            return None;
        }
        // If we definitely have an arrow function, then we can just parse one, not requiring a
        // following => or { token. Otherwise, we *might* have an arrow function.  Try to parse
        // it out, but don't allow any ambiguity, and return 'undefined' if this could be an
        // expression instead.
        if tristate == Tristate::True {
            return self.parse_parenthesized_arrow_function_expression(true /*allowAmbiguity*/, true /*allowReturnTypeInArrowFunction*/);
        }
        let state = self.mark();
        let result = self.parse_possible_parenthesized_arrow_function_expression(allow_return_type_in_arrow_function);
        if result.is_none() {
            self.rewind(state);
        }
        result
    }

    pub(crate) fn parse_parenthesized_arrow_function_expression(&mut self, allow_ambiguity: bool, allow_return_type_in_arrow_function: bool) -> Option<P<Node>> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        let modifiers = self.parse_modifiers_for_arrow_function();
        let is_async = modifier_list_has_async(modifiers);
        let signature_flags = if is_async { ParseFlags::Await } else { ParseFlags::None };
        // Arrow functions are never generators.
        //
        // If we're speculatively parsing a signature for a parenthesized arrow function, then
        // we have to have a complete parameter list.  Otherwise we might see something like
        // a => (b => c)
        // And think that "(b =>" was actually a parenthesized arrow function with a missing
        // close paren.
        let type_parameters = self.parse_type_parameters();
        let parameters: P<NodeList>;
        if !self.parse_expected(Kind::OpenParenToken) {
            if !allow_ambiguity {
                return None;
            }
            parameters = self.create_missing_list();
        } else {
            if !allow_ambiguity {
                let maybe_parameters = self.parse_parameters_worker(signature_flags, allow_ambiguity);
                match maybe_parameters {
                    None => return None,
                    Some(maybe_parameters) => parameters = maybe_parameters,
                }
            } else {
                parameters = self.parse_parameters_worker(signature_flags, allow_ambiguity).unwrap();
            }
            if !self.parse_expected(Kind::CloseParenToken) && !allow_ambiguity {
                return None;
            }
        }
        let has_return_colon = self.token == Kind::ColonToken;
        let return_type = self.parse_return_type(Kind::ColonToken, false /*isType*/);
        if let Some(return_type) = return_type {
            if !allow_ambiguity && type_has_arrow_function_blocking_parse_error(return_type) {
                return None;
            }
        }
        // Parsing a signature isn't enough.
        // Parenthesized arrow signatures often look like other valid expressions.
        // For instance:
        //  - "(x = 10)" is an assignment expression parsed as a signature with a default parameter value.
        //  - "(x,y)" is a comma expression parsed as a signature with two parameters.
        //  - "a ? (b): c" will have "(b):" parsed as a signature with a return type annotation.
        //  - "a ? (b): function() {}" will too, since function() is a valid JSDoc function type.
        //  - "a ? (b): (function() {})" as well, but inside of a parenthesized type with an arbitrary amount of nesting.
        //
        // So we need just a bit of lookahead to ensure that it can only be a signature.
        let mut unwrapped_type = return_type;
        while let Some(t) = unwrapped_type {
            if t.kind != Kind::ParenthesizedType {
                break;
            }
            unwrapped_type = t.type_node(); // Skip parens if need be
        }
        if !allow_ambiguity && self.token != Kind::EqualsGreaterThanToken && self.token != Kind::OpenBraceToken {
            // Returning undefined here will cause our caller to rewind to where we started from.
            return None;
        }
        // If we have an arrow, then try to parse the body. Even if not, try to parse if we
        // have an opening brace, just in case we're in an error state.
        let last_token = self.token;
        let equals_greater_than_token = self.parse_expected_token(Kind::EqualsGreaterThanToken);
        let body = if last_token == Kind::EqualsGreaterThanToken || last_token == Kind::OpenBraceToken {
            self.parse_arrow_function_expression_body(is_async, allow_return_type_in_arrow_function)
        } else {
            self.parse_identifier()
        };
        // Given:
        //     x ? y => ({ y }) : z => ({ z })
        // We try to parse the body of the first arrow function by looking at:
        //     ({ y }) : z => ({ z })
        // This is a valid arrow function with "z" as the return type.
        //
        // But, if we're in the true side of a conditional expression, this colon
        // terminates the expression, so we cannot allow a return type if we aren't
        // certain whether or not the preceding text was parsed as a parameter list.
        //
        // For example,
        //     a() ? (b: number, c?: string): void => d() : e
        // is determined by isParenthesizedArrowFunctionExpression to unambiguously
        // be an arrow expression, so we allow a return type.
        if !allow_return_type_in_arrow_function && has_return_colon {
            // However, if the arrow function we were able to parse is followed by another colon
            // as in:
            //     a ? (x): string => x : null
            // Then allow the arrow function, and treat the second colon as terminating
            // the conditional expression. It's okay to do this because this code would
            // be a syntax error in JavaScript (as the second colon shouldn't be there).
            if self.token != Kind::ColonToken {
                return None;
            }
        }
        let node = self.factory.new_arrow_function(modifiers, type_parameters, parameters, return_type, None /*fullSignature*/, equals_greater_than_token, body);
        let result = self.finish_node(node, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        Some(result)
    }

    pub(crate) fn parse_modifiers_for_arrow_function(&mut self) -> Option<P<ModifierList>> {
        if self.token == Kind::AsyncKeyword {
            let pos = self.node_pos();
            self.next_token();
            let node = self.factory.new_modifier(Kind::AsyncKeyword);
            let modifier = self.finish_node(node, pos);
            return Some(self.new_modifier_list(modifier.loc(), &[modifier]));
        }
        None
    }

    pub(crate) fn parse_arrow_function_expression_body(&mut self, is_async: bool, allow_return_type_in_arrow_function: bool) -> P<Node> {
        if self.token == Kind::OpenBraceToken {
            return self.parse_function_block(if is_async { ParseFlags::Await } else { ParseFlags::None }, None /*diagnosticMessage*/);
        }
        if self.token != Kind::SemicolonToken && self.token != Kind::FunctionKeyword && self.token != Kind::ClassKeyword && self.is_start_of_statement() && !self.is_start_of_expression_statement() {
            // Check if we got a plain statement (i.e. no expression-statements, no function/class expressions/declarations)
            //
            // Here we try to recover from a potential error situation in the case where the
            // user meant to supply a block. For example, if the user wrote:
            //
            //  a =>
            //      let v = 0;
            //  }
            //
            // they may be missing an open brace.  Check to see if that's the case so we can
            // try to recover better.  If we don't do this, then the next close curly we see may end
            // up preemptively closing the containing construct.
            //
            // Note: even when 'IgnoreMissingOpenBrace' is passed, parseBody will still error.
            return self.parse_function_block(ParseFlags::IgnoreMissingOpenBrace | if is_async { ParseFlags::Await } else { ParseFlags::None }, None /*diagnosticMessage*/);
        }
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::AwaitContext, is_async);
        self.set_context_flags(NodeFlags::YieldContext, false);
        let node = self.parse_assignment_expression_or_higher_worker(allow_return_type_in_arrow_function);
        self.context_flags = save_context_flags;
        node
    }

    pub(crate) fn is_start_of_expression_statement(&mut self) -> bool {
        // As per the grammar, none of '{' or 'function' or 'class' can start an expression statement.
        self.token != Kind::OpenBraceToken && self.token != Kind::FunctionKeyword && self.token != Kind::ClassKeyword && self.token != Kind::AtToken && self.is_start_of_expression()
    }

    pub(crate) fn parse_possible_parenthesized_arrow_function_expression(&mut self, allow_return_type_in_arrow_function: bool) -> Option<P<Node>> {
        let token_pos = self.scanner.token_start();
        if self.not_parenthesized_arrow.contains(&token_pos) {
            return None;
        }
        let result = self.parse_parenthesized_arrow_function_expression(false /*allowAmbiguity*/, allow_return_type_in_arrow_function);
        if result.is_none() {
            self.not_parenthesized_arrow.insert(token_pos);
        }
        result
    }

    pub(crate) fn try_parse_async_simple_arrow_function_expression(&mut self, allow_return_type_in_arrow_function: bool) -> Option<P<Node>> {
        // We do a check here so that we won't be doing unnecessarily call to "lookAhead"
        if self.token == Kind::AsyncKeyword && self.look_ahead(Parser::next_is_un_parenthesized_async_arrow_function) {
            let pos = self.node_pos();
            let jsdoc = self.jsdoc_scanner_info();
            let async_modifier = self.parse_modifiers_for_arrow_function();
            let expr = self.parse_binary_expression_or_higher(OperatorPrecedence::Lowest);
            return Some(self.parse_simple_arrow_function_expression(pos, expr, allow_return_type_in_arrow_function, jsdoc, async_modifier));
        }
        None
    }

    pub(crate) fn next_is_un_parenthesized_async_arrow_function(&mut self) -> bool {
        // AsyncArrowFunctionExpression:
        //      1) async[no LineTerminator here]AsyncArrowBindingIdentifier[?Yield][no LineTerminator here]=>AsyncConciseBody[?In]
        //      2) CoverCallExpressionAndAsyncArrowHead[?Yield, ?Await][no LineTerminator here]=>AsyncConciseBody[?In]
        if self.token == Kind::AsyncKeyword {
            self.next_token();
            // If the "async" is followed by "=>" token then it is not a beginning of an async arrow-function
            // but instead a simple arrow-function which will be parsed inside "parseAssignmentExpressionOrHigher"
            if self.has_preceding_line_break() || self.token == Kind::EqualsGreaterThanToken {
                return false;
            }
            // Check for un-parenthesized AsyncArrowFunction
            if !self.is_identifier() {
                return false;
            }
            self.next_token_without_check();
            return !self.has_preceding_line_break() && self.token == Kind::EqualsGreaterThanToken;
        }
        false
    }

    pub(crate) fn parse_simple_arrow_function_expression(&mut self, pos: i32, identifier: P<Node>, allow_return_type_in_arrow_function: bool, jsdoc: JsdocScannerInfo, async_modifier: Option<P<ModifierList>>) -> P<Node> {
        assert!(self.token == Kind::EqualsGreaterThanToken, "parseSimpleArrowFunctionExpression should only have been called if we had a =>");
        let node = self.factory.new_parameter_declaration(None /*modifiers*/, None /*dotDotDotToken*/, identifier, None /*questionToken*/, None /*typeNode*/, None /*initializer*/);
        let parameter = self.finish_node(node, identifier.pos());
        let parameters = self.new_node_list(parameter.loc(), &[parameter]);
        let equals_greater_than_token = self.parse_expected_token(Kind::EqualsGreaterThanToken);
        let body = self.parse_arrow_function_expression_body(async_modifier.is_some() /*isAsync*/, allow_return_type_in_arrow_function);
        let node = self.factory.new_arrow_function(async_modifier, None /*typeParameters*/, parameters, None /*returnType*/, None /*fullSignature*/, equals_greater_than_token, body);
        let result = self.finish_node(node, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_conditional_expression_rest(&mut self, left_operand: P<Node>, pos: i32, allow_return_type_in_arrow_function: bool) -> P<Node> {
        // Note: we are passed in an expression which was produced from parseBinaryExpressionOrHigher.
        let Some(question_token) = self.parse_optional_token(Kind::QuestionToken) else {
            return left_operand;
        };
        // Note: we explicitly 'allowIn' in the whenTrue part of the condition expression, and
        // we do not that for the 'whenFalse' part.
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::DisallowInContext, false);
        let true_expression = self.parse_assignment_expression_or_higher_worker(false /*allowReturnTypeInArrowFunction*/);
        self.context_flags = save_context_flags;
        let colon_token = self.parse_expected_token(Kind::ColonToken);
        let false_expression = if ast::node_is_present(Some(colon_token)) {
            self.parse_assignment_expression_or_higher_worker(allow_return_type_in_arrow_function)
        } else {
            self.create_missing_identifier()
        };
        let node = self.factory.new_conditional_expression(left_operand, question_token, true_expression, colon_token, false_expression);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_binary_expression_or_higher(&mut self, precedence: OperatorPrecedence) -> P<Node> {
        let pos = self.node_pos();
        let left_operand = self.parse_unary_expression_or_higher();
        self.parse_binary_expression_rest(precedence, left_operand, pos)
    }

    pub(crate) fn parse_binary_expression_rest(&mut self, precedence: OperatorPrecedence, left_operand: P<Node>, pos: i32) -> P<Node> {
        let mut left_operand = left_operand;
        let mut last_operand = left_operand;
        loop {
            // We either have a binary operator here, or we're finished.  We call
            // reScanGreaterToken so that we merge token sequences like > and = into >=
            let operator = self.re_scan_greater_than_token();
            let new_precedence = ast::get_binary_operator_precedence(operator);
            // Check the precedence to see if we should "take" this operator
            // - For left associative operator (all operator but **), consume the operator,
            //   recursively call the function below, and parse binaryExpression as a rightOperand
            //   of the caller if the new precedence of the operator is greater then or equal to the current precedence.
            //   For example:
            //      a - b - c;
            //            ^token; leftOperand = b. Return b to the caller as a rightOperand
            //      a * b - c
            //            ^token; leftOperand = b. Return b to the caller as a rightOperand
            //      a - b * c;
            //            ^token; leftOperand = b. Return b * c to the caller as a rightOperand
            // - For right associative operator (**), consume the operator, recursively call the function
            //   and parse binaryExpression as a rightOperand of the caller if the new precedence of
            //   the operator is strictly grater than the current precedence
            //   For example:
            //      a ** b ** c;
            //             ^^token; leftOperand = b. Return b ** c to the caller as a rightOperand
            //      a - b ** c;
            //            ^^token; leftOperand = b. Return b ** c to the caller as a rightOperand
            //      a ** b - c
            //             ^token; leftOperand = b. Return b to the caller as a rightOperand
            if !should_consume_binary_operator(operator, new_precedence, precedence) {
                break;
            }
            if operator == Kind::InKeyword && self.in_disallow_in_context() {
                break;
            }
            if operator == Kind::AsKeyword || operator == Kind::SatisfiesKeyword {
                // Make sure we *do* perform ASI for constructs like this:
                //    var x = foo
                //    as (Bar)
                // This should be parsed as an initialized variable, followed
                // by a function call to 'as' with the argument 'Bar'
                if self.has_preceding_line_break() {
                    break;
                } else {
                    self.next_token();
                    // When we have 'a ## b as SomeType $$ c' or 'a ## b satisfies SomeType $$ c', where ## and $$
                    // are binary operators, we want to stop parsing when $$ would bind before ## after erasing the
                    // assertion. See https://github.com/microsoft/TypeScript/issues/63527.
                    let mut last_precedence = OperatorPrecedence::Highest;
                    if ast::is_binary_expression(last_operand) {
                        last_precedence = ast::get_binary_operator_precedence(last_operand.as_binary_expression().operator_token.kind);
                    }
                    if operator == Kind::SatisfiesKeyword {
                        let type_node = self.parse_type();
                        left_operand = self.make_satisfies_expression(left_operand, type_node);
                    } else {
                        let type_node = self.parse_type();
                        left_operand = self.make_as_expression(left_operand, type_node);
                    }
                    // Stop if the next operator would bind before the last operator when the assertion is erased.
                    let next_operator = self.re_scan_greater_than_token();
                    let next_precedence = ast::get_binary_operator_precedence(next_operator);
                    if should_consume_binary_operator(next_operator, next_precedence, last_precedence) {
                        break;
                    }
                }
            } else {
                let operator_token = self.parse_token_node();
                let right = self.parse_binary_expression_or_higher(new_precedence);
                left_operand = self.make_binary_expression(left_operand, operator_token, right, pos);
                last_operand = left_operand;
            }
        }
        left_operand
    }

    pub(crate) fn make_satisfies_expression(&mut self, expression: P<Node>, type_node: P<Node>) -> P<Node> {
        let node = self.factory.new_satisfies_expression(expression, type_node);
        let node = self.finish_node(node, expression.pos());
        self.check_js_syntax(node)
    }

    pub(crate) fn make_as_expression(&mut self, left: P<Node>, right: P<Node>) -> P<Node> {
        let node = self.factory.new_as_expression(left, right);
        let node = self.finish_node(node, left.pos());
        self.check_js_syntax(node)
    }

    pub(crate) fn make_binary_expression(&mut self, left: P<Node>, operator_token: P<Node>, right: P<Node>, pos: i32) -> P<Node> {
        let node = self.factory.new_binary_expression(None /*modifiers*/, left, None /*typeNode*/, operator_token, right);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_unary_expression_or_higher(&mut self) -> P<Node> {
        // ES7 UpdateExpression:
        //      1) LeftHandSideExpression[?Yield]
        //      2) LeftHandSideExpression[?Yield][no LineTerminator here]++
        //      3) LeftHandSideExpression[?Yield][no LineTerminator here]--
        //      4) ++UnaryExpression[?Yield]
        //      5) --UnaryExpression[?Yield]
        if self.is_update_expression() {
            let pos = self.node_pos();
            let update_expression = self.parse_update_expression();
            if self.token == Kind::AsteriskAsteriskToken {
                return self.parse_binary_expression_rest(ast::get_binary_operator_precedence(self.token), update_expression, pos);
            }
            return update_expression;
        }
        // ES7 UnaryExpression:
        //      1) UpdateExpression[?yield]
        //      2) delete UpdateExpression[?yield]
        //      3) void UpdateExpression[?yield]
        //      4) typeof UpdateExpression[?yield]
        //      5) + UpdateExpression[?yield]
        //      6) - UpdateExpression[?yield]
        //      7) ~ UpdateExpression[?yield]
        //      8) ! UpdateExpression[?yield]
        let unary_operator = self.token;
        let simple_unary_expression = self.parse_simple_unary_expression();
        if self.token == Kind::AsteriskAsteriskToken {
            let pos = scanner::skip_trivia(self.source_text, simple_unary_expression.pos());
            let end = simple_unary_expression.end();
            if simple_unary_expression.kind == Kind::TypeAssertionExpression {
                self.parse_error_at(pos, end, &diagnostics::A_type_assertion_expression_is_not_allowed_in_the_left_hand_side_of_an_exponentiation_expression_Consider_enclosing_the_expression_in_parentheses, &[]);
            } else {
                assert!(is_keyword_or_punctuation(unary_operator));
                self.parse_error_at(pos, end, &diagnostics::An_unary_expression_with_the_0_operator_is_not_allowed_in_the_left_hand_side_of_an_exponentiation_expression_Consider_enclosing_the_expression_in_parentheses, &[&scanner::token_to_string(unary_operator)]);
            }
        }
        simple_unary_expression
    }

    pub(crate) fn is_update_expression(&mut self) -> bool {
        match self.token {
            Kind::PlusToken | Kind::MinusToken | Kind::TildeToken | Kind::ExclamationToken | Kind::DeleteKeyword | Kind::TypeOfKeyword | Kind::VoidKeyword | Kind::AwaitKeyword => false,
            Kind::LessThanToken => self.language_variant == LanguageVariant::JSX,
            _ => true,
        }
    }

    pub(crate) fn parse_update_expression(&mut self) -> P<Node> {
        let pos = self.node_pos();
        if self.token == Kind::PlusPlusToken || self.token == Kind::MinusMinusToken {
            let operator = self.token;
            self.next_token();
            let operand = self.parse_left_hand_side_expression_or_higher();
            let node = self.factory.new_prefix_unary_expression(operator, operand);
            return self.finish_node(node, pos);
        } else if self.language_variant == LanguageVariant::JSX && self.token == Kind::LessThanToken && self.look_ahead(Parser::next_token_is_identifier_or_keyword_or_greater_than) {
            // JSXElement is part of primaryExpression
            return self.parse_jsx_element_or_self_closing_element_or_fragment(true /*inExpressionContext*/, -1 /*topInvalidNodePosition*/, None /*openingTag*/, false /*mustBeUnary*/);
        }
        let expression = self.parse_left_hand_side_expression_or_higher();
        if (self.token == Kind::PlusPlusToken || self.token == Kind::MinusMinusToken) && !self.has_preceding_line_break() {
            let operator = self.token;
            self.next_token();
            let node = self.factory.new_postfix_unary_expression(expression, operator);
            return self.finish_node(node, pos);
        }
        expression
    }

    pub(crate) fn parse_jsx_element_or_self_closing_element_or_fragment(&mut self, in_expression_context: bool, top_invalid_node_position: i32, opening_tag: Option<P<Node>>, must_be_unary: bool) -> P<Node> {
        let pos = self.node_pos();
        let opening = self.parse_jsx_opening_or_self_closing_element_or_opening_fragment(in_expression_context);
        let mut result: P<Node>;
        match opening.kind {
            Kind::JsxOpeningElement => {
                let mut children = self.parse_jsx_children(opening);
                let closing_element: P<Node>;
                let last_child = children.nodes.last().copied();
                if let Some(last_child) = last_child.filter(|last_child| {
                    last_child.kind == Kind::JsxElement
                        && !ast::tag_names_are_equivalent(last_child.as_jsx_element().opening_element.tag_name().unwrap(), last_child.as_jsx_element().closing_element.tag_name().unwrap())
                        && ast::tag_names_are_equivalent(opening.tag_name().unwrap(), last_child.as_jsx_element().closing_element.tag_name().unwrap())
                }) {
                    // when an unclosed JsxOpeningElement incorrectly parses its parent's JsxClosingElement,
                    // restructure (<div>(...<span>...</div>)) --> (<div>(...<span>...</>)</div>)
                    // (no need to error; the parent will error)
                    let last_child_data = last_child.as_jsx_element();
                    let end = last_child_data.children.end();
                    let missing = self.new_identifier("");
                    let missing_identifier = self.finish_node_with_end(missing, end, end);
                    let closing = self.factory.new_jsx_closing_element(missing_identifier);
                    let new_closing_element = self.finish_node_with_end(closing, end, end);
                    let element = self.factory.new_jsx_element(last_child_data.opening_element, last_child_data.children, new_closing_element);
                    let new_last = self.finish_node_with_end(element, last_child_data.opening_element.pos(), end);
                    // force reset parent pointers from discarded parse result
                    last_child_data.opening_element.set_parent(Some(new_last));
                    for c in last_child_data.children.nodes {
                        c.set_parent(Some(new_last));
                    }
                    new_closing_element.set_parent(Some(new_last));
                    let mut nodes = children.nodes[0..children.nodes.len() - 1].to_vec();
                    nodes.push(new_last);
                    children = self.new_node_list(TextRange::new(children.pos(), new_last.end()), &nodes);
                    closing_element = last_child_data.closing_element;
                } else {
                    closing_element = self.parse_jsx_closing_element(opening, in_expression_context);
                    if !ast::tag_names_are_equivalent(opening.tag_name().unwrap(), closing_element.tag_name().unwrap()) {
                        if opening_tag.is_some_and(|opening_tag| ast::is_jsx_opening_element(opening_tag) && ast::tag_names_are_equivalent(closing_element.tag_name().unwrap(), opening_tag.tag_name().unwrap())) {
                            // opening incorrectly matched with its parent's closing -- put error on opening
                            let text = scanner::get_text_of_node_from_source_text(self.source_text, opening.tag_name().unwrap(), false /*includeTrivia*/);
                            self.parse_error_at_range(opening.tag_name().unwrap().loc(), &diagnostics::JSX_element_0_has_no_corresponding_closing_tag, &[&text]);
                        } else {
                            // other opening/closing mismatches -- put error on closing
                            let text = scanner::get_text_of_node_from_source_text(self.source_text, opening.tag_name().unwrap(), false /*includeTrivia*/);
                            self.parse_error_at_range(closing_element.tag_name().unwrap().loc(), &diagnostics::Expected_corresponding_JSX_closing_tag_for_0, &[&text]);
                        }
                    }
                }
                let element = self.factory.new_jsx_element(opening, children, closing_element);
                result = self.finish_node(element, pos);
                closing_element.set_parent(Some(result)); // force reset parent pointers from possibly discarded parse result
            }
            Kind::JsxOpeningFragment => {
                let children = self.parse_jsx_children(opening);
                let closing_fragment = self.parse_jsx_closing_fragment(in_expression_context);
                let fragment = self.factory.new_jsx_fragment(opening, children, closing_fragment);
                result = self.finish_node(fragment, pos);
            }
            Kind::JsxSelfClosingElement => {
                // Nothing else to do for self-closing elements
                result = opening;
            }
            _ => panic!("Unhandled case in parseJsxElementOrSelfClosingElementOrFragment"),
        }
        // If the user writes the invalid code '<div></div><div></div>' in an expression context (i.e. not wrapped in
        // an enclosing tag), we'll naively try to parse   ^ this as a 'less than' operator and the remainder of the tag
        // as garbage, which will cause the formatter to badly mangle the JSX. Perform a speculative parse of a JSX
        // element if we see a < token so that we can wrap it in a synthetic binary expression so the formatter
        // does less damage and we can report a better error.
        // Since JSX elements are invalid < operands anyway, this lookahead parse will only occur in error scenarios
        // of one sort or another.
        // If we are in a unary context, we can't do this recovery; the binary expression we return here is not
        // a valid UnaryExpression and will cause problems later.
        if !must_be_unary && in_expression_context && self.token == Kind::LessThanToken {
            let mut top_bad_pos = top_invalid_node_position;
            if top_bad_pos < 0 {
                top_bad_pos = result.pos();
            }
            let invalid_element = self.parse_jsx_element_or_self_closing_element_or_fragment(true /*inExpressionContext*/, top_bad_pos, None, false);
            let operator_token = self.factory.new_token(Kind::CommaToken);
            operator_token.set_loc(TextRange::new(invalid_element.pos(), invalid_element.pos()));
            self.parse_error_at(scanner::skip_trivia(self.source_text, top_bad_pos), invalid_element.end(), &diagnostics::JSX_expressions_must_have_one_parent_element, &[]);
            let binary = self.factory.new_binary_expression(None /*modifiers*/, result, None /*typeNode*/, operator_token, invalid_element);
            result = self.finish_node(binary, pos);
        }
        result
    }

    pub(crate) fn parse_jsx_children(&mut self, opening_tag: P<Node>) -> P<NodeList> {
        let pos = self.node_pos();
        let save_parsing_contexts = self.parsing_contexts;
        self.parsing_contexts |= 1 << (ParsingContext::JsxChildren as i32);
        let mut list: Vec<P<Node>> = Vec::new();
        loop {
            let current_token = self.scanner.re_scan_jsx_token(true /*allowMultilineJsxText*/);
            let Some(child) = self.parse_jsx_child(opening_tag, current_token) else {
                break;
            };
            list.push(child);
            if ast::is_jsx_opening_element(opening_tag)
                && child.kind == Kind::JsxElement
                && !ast::tag_names_are_equivalent(child.as_jsx_element().opening_element.tag_name().unwrap(), child.as_jsx_element().closing_element.tag_name().unwrap())
                && ast::tag_names_are_equivalent(opening_tag.tag_name().unwrap(), child.as_jsx_element().closing_element.tag_name().unwrap())
            {
                // stop after parsing a mismatched child like <div>...(<span></div>) in order to reattach the </div> higher
                break;
            }
        }
        self.parsing_contexts = save_parsing_contexts;
        let end = self.node_pos();
        self.new_node_list(TextRange::new(pos, end), &list)
    }

    pub(crate) fn parse_jsx_child(&mut self, opening_tag: P<Node>, token: Kind) -> Option<P<Node>> {
        match token {
            Kind::EndOfFile => {
                // If we hit EOF, issue the error at the tag that lacks the closing element
                // rather than at the end of the file (which is useless)
                if ast::is_jsx_opening_fragment(opening_tag) {
                    self.parse_error_at_range(opening_tag.loc(), &diagnostics::JSX_fragment_has_no_corresponding_closing_tag, &[]);
                } else {
                    // We want the error span to cover only 'Foo.Bar' in < Foo.Bar >
                    // or to cover only 'Foo' in < Foo >
                    let tag = opening_tag.tag_name().unwrap();
                    let start = std::cmp::min(scanner::skip_trivia(self.source_text, tag.pos()), tag.end());
                    let text = scanner::get_text_of_node_from_source_text(self.source_text, opening_tag.tag_name().unwrap(), false /*includeTrivia*/);
                    self.parse_error_at(start, tag.end(), &diagnostics::JSX_element_0_has_no_corresponding_closing_tag, &[&text]);
                }
                None
            }
            Kind::LessThanSlashToken | Kind::ConflictMarkerTrivia => None,
            Kind::JsxText | Kind::JsxTextAllWhiteSpaces => Some(self.parse_jsx_text()),
            Kind::OpenBraceToken => self.parse_jsx_expression(false /*inExpressionContext*/),
            Kind::LessThanToken => Some(self.parse_jsx_element_or_self_closing_element_or_fragment(false /*inExpressionContext*/, -1 /*topInvalidNodePosition*/, Some(opening_tag), false)),
            _ => panic!("Unhandled case in parseJsxChild"),
        }
    }

    pub(crate) fn parse_jsx_text(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let text = tsrs_core::alloc_str(&self.scanner.token_value());
        let result = self.factory.new_jsx_text(text, self.token == Kind::JsxTextAllWhiteSpaces);
        self.scan_jsx_text();
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_jsx_expression(&mut self, in_expression_context: bool) -> Option<P<Node>> {
        let pos = self.node_pos();
        if !self.parse_expected(Kind::OpenBraceToken) {
            return None;
        }
        let mut dot_dot_dot_token: Option<P<Node>> = None;
        let mut expression: Option<P<Node>> = None;
        if self.token != Kind::CloseBraceToken {
            if !in_expression_context {
                dot_dot_dot_token = self.parse_optional_token(Kind::DotDotDotToken);
            }
            // Only an AssignmentExpression is valid here per the JSX spec,
            // but we can unambiguously parse a comma sequence and provide
            // a better error message in grammar checking.
            expression = Some(self.parse_expression());
        }
        if in_expression_context {
            self.parse_expected(Kind::CloseBraceToken);
        } else if self.parse_expected_without_advancing(Kind::CloseBraceToken) {
            self.scan_jsx_text();
        }
        let node = self.factory.new_jsx_expression(dot_dot_dot_token, expression);
        Some(self.finish_node(node, pos))
    }

    pub(crate) fn scan_jsx_text(&mut self) -> Kind {
        self.token = self.scanner.scan_jsx_token();
        self.token
    }

    pub(crate) fn scan_jsx_identifier(&mut self) -> Kind {
        self.token = self.scanner.scan_jsx_identifier();
        self.token
    }

    pub(crate) fn scan_jsx_attribute_value(&mut self) -> Kind {
        self.token = self.scanner.scan_jsx_attribute_value();
        self.token
    }

    pub(crate) fn parse_jsx_closing_element(&mut self, open: P<Node>, in_expression_context: bool) -> P<Node> {
        let pos = self.node_pos();
        self.parse_expected(Kind::LessThanSlashToken);
        let tag_name = self.parse_jsx_element_name();
        if self.parse_expected_with_diagnostic(Kind::GreaterThanToken, None /*diagnosticMessage*/, false /*shouldAdvance*/) {
            // manually advance the scanner in order to look for jsx text inside jsx
            if in_expression_context || !ast::tag_names_are_equivalent(open.tag_name().unwrap(), tag_name) {
                self.next_token();
            } else {
                self.scan_jsx_text();
            }
        }
        let node = self.factory.new_jsx_closing_element(tag_name);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_jsx_opening_or_self_closing_element_or_opening_fragment(&mut self, in_expression_context: bool) -> P<Node> {
        let pos = self.node_pos();
        self.parse_expected(Kind::LessThanToken);
        if self.token == Kind::GreaterThanToken {
            // See below for explanation of scanJsxText
            self.scan_jsx_text();
            let node = self.factory.new_jsx_opening_fragment();
            return self.finish_node(node, pos);
        }
        let tag_name = self.parse_jsx_element_name();
        let mut type_arguments: Option<P<NodeList>> = None;
        if !self.context_flags.intersects(NodeFlags::JavaScriptFile) {
            type_arguments = self.parse_type_arguments();
        }
        let attributes = self.parse_jsx_attributes();
        let result;
        if self.token == Kind::GreaterThanToken {
            // Closing tag, so scan the immediately-following text with the JSX scanning instead
            // of regular scanning to avoid treating illegal characters (e.g. '#') as immediate
            // scanning errors
            self.scan_jsx_text();
            result = self.factory.new_jsx_opening_element(tag_name, type_arguments, attributes);
        } else {
            self.parse_expected(Kind::SlashToken);
            if self.parse_expected_without_advancing(Kind::GreaterThanToken) {
                if in_expression_context {
                    self.next_token();
                } else {
                    self.scan_jsx_text();
                }
            }
            result = self.factory.new_jsx_self_closing_element(tag_name, type_arguments, attributes);
        }
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_jsx_element_name(&mut self) -> P<Node> {
        let pos = self.node_pos();
        // JsxElement can have name in the form of
        //      propertyAccessExpression
        //      primaryExpression in the form of an identifier and "this" keyword
        // We can't just simply use parseLeftHandSideExpressionOrHigher because then we will start consider class,function etc as a keyword
        // We only want to consider "this" as a primaryExpression
        let initial_expression = self.parse_jsx_tag_name();
        if ast::is_jsx_namespaced_name(initial_expression) {
            return initial_expression; // `a:b.c` is invalid syntax, don't even look for the `.` if we parse `a:b`, and let `parseAttribute` report "unexpected :" instead.
        }
        let mut expression = initial_expression;
        while self.parse_optional(Kind::DotToken) {
            let name = self.parse_right_side_of_dot(true /*allowIdentifierNames*/, false /*allowPrivateIdentifiers*/, false /*allowUnicodeEscapeSequenceInIdentifierName*/);
            let node = self.factory.new_property_access_expression(expression, None, name, NodeFlags::None);
            expression = self.finish_node(node, pos);
        }
        expression
    }

    pub(crate) fn parse_jsx_tag_name(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.scan_jsx_identifier();
        let is_this = self.token == Kind::ThisKeyword;
        let tag_name = self.parse_identifier_name_error_on_unicode_escape_sequence();
        if self.parse_optional(Kind::ColonToken) {
            self.scan_jsx_identifier();
            let name = self.parse_identifier_name_error_on_unicode_escape_sequence();
            let node = self.factory.new_jsx_namespaced_name(tag_name, name);
            return self.finish_node(node, pos);
        }
        if is_this {
            let result = self.factory.new_keyword_expression(Kind::ThisKeyword);
            return self.finish_node(result, pos);
        }
        tag_name
    }

    pub(crate) fn parse_jsx_attributes(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let properties = self.parse_list(ParsingContext::JsxAttributes, Parser::parse_jsx_attribute);
        let node = self.factory.new_jsx_attributes(properties);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_jsx_attribute(&mut self) -> P<Node> {
        if self.token == Kind::OpenBraceToken {
            return self.parse_jsx_spread_attribute();
        }
        let pos = self.node_pos();
        let name = self.parse_jsx_attribute_name();
        let initializer = self.parse_jsx_attribute_value();
        let node = self.factory.new_jsx_attribute(name, initializer);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_jsx_spread_attribute(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.parse_expected(Kind::OpenBraceToken);
        self.parse_expected(Kind::DotDotDotToken);
        let expression = self.parse_expression();
        self.parse_expected(Kind::CloseBraceToken);
        let node = self.factory.new_jsx_spread_attribute(expression);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_jsx_attribute_name(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.scan_jsx_identifier();
        let attr_name = self.parse_identifier_name_error_on_unicode_escape_sequence();
        if self.parse_optional(Kind::ColonToken) {
            self.scan_jsx_identifier();
            let name = self.parse_identifier_name_error_on_unicode_escape_sequence();
            let node = self.factory.new_jsx_namespaced_name(attr_name, name);
            return self.finish_node(node, pos);
        }
        attr_name
    }

    pub(crate) fn parse_jsx_attribute_value(&mut self) -> Option<P<Node>> {
        if self.token == Kind::EqualsToken {
            if self.scan_jsx_attribute_value() == Kind::StringLiteral {
                return Some(self.parse_literal_expression());
            }
            if self.token == Kind::OpenBraceToken {
                return self.parse_jsx_expression(true /*inExpressionContext*/);
            }
            if self.token == Kind::LessThanToken {
                // An attribute value must be a single JsxAttributeValue, so don't allow the sibling-element
                // recovery to wrap it in a synthetic binary expression.
                return Some(self.parse_jsx_element_or_self_closing_element_or_fragment(true /*inExpressionContext*/, -1 /*topInvalidNodePosition*/, None /*openingTag*/, true /*mustBeUnary*/));
            }
            self.parse_error_at_current_token(&diagnostics::X_or_JSX_element_expected, &[]);
        }
        None
    }

    pub(crate) fn parse_jsx_closing_fragment(&mut self, in_expression_context: bool) -> P<Node> {
        let pos = self.node_pos();
        self.parse_expected(Kind::LessThanSlashToken);
        if self.parse_expected_with_diagnostic(Kind::GreaterThanToken, Some(&diagnostics::Expected_corresponding_closing_tag_for_JSX_fragment), false /*shouldAdvance*/) {
            // manually advance the scanner in order to look for jsx text inside jsx
            if in_expression_context {
                self.next_token();
            } else {
                self.scan_jsx_text();
            }
        }
        let node = self.factory.new_jsx_closing_fragment();
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_simple_unary_expression(&mut self) -> P<Node> {
        let token = self.token;
        match token {
            Kind::PlusToken | Kind::MinusToken | Kind::TildeToken | Kind::ExclamationToken => self.parse_prefix_unary_expression(),
            Kind::DeleteKeyword => self.parse_delete_expression(),
            Kind::TypeOfKeyword => self.parse_type_of_expression(),
            Kind::VoidKeyword => self.parse_void_expression(),
            Kind::LessThanToken => {
                // Just like in parseUpdateExpression, we need to avoid parsing type assertions when
                // in JSX and we see an expression like "+ <foo> bar".
                if self.language_variant == LanguageVariant::JSX {
                    return self.parse_jsx_element_or_self_closing_element_or_fragment(true /*inExpressionContext*/, -1 /*topInvalidNodePosition*/, None /*openingTag*/, true /*mustBeUnary*/);
                }
                // // This is modified UnaryExpression grammar in TypeScript
                // //  UnaryExpression (modified):
                // //      < type > UnaryExpression
                self.parse_type_assertion()
            }
            Kind::AwaitKeyword if self.is_await_expression() => self.parse_await_expression(),
            _ => self.parse_update_expression(),
        }
    }

    pub(crate) fn parse_prefix_unary_expression(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let operator = self.token;
        self.next_token();
        let operand = self.parse_simple_unary_expression();
        let node = self.factory.new_prefix_unary_expression(operator, operand);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_delete_expression(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.next_token();
        let expression = self.parse_simple_unary_expression();
        let node = self.factory.new_delete_expression(expression);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_type_of_expression(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.next_token();
        let expression = self.parse_simple_unary_expression();
        let node = self.factory.new_type_of_expression(expression);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_void_expression(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.next_token();
        let expression = self.parse_simple_unary_expression();
        let node = self.factory.new_void_expression(expression);
        self.finish_node(node, pos)
    }

    pub(crate) fn is_await_expression(&mut self) -> bool {
        if self.token == Kind::AwaitKeyword {
            if self.in_await_context() {
                return true;
            }
            // here we are using similar heuristics as 'isYieldExpression'
            return self.look_ahead(Parser::next_token_is_identifier_or_keyword_or_literal_on_same_line);
        }
        false
    }

    pub(crate) fn parse_await_expression(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.next_token();
        let expression = self.parse_simple_unary_expression();
        let node = self.factory.new_await_expression(expression);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_type_assertion(&mut self) -> P<Node> {
        assert!(self.language_variant != LanguageVariant::JSX, "Type assertions should never be parsed in JSX; they should be parsed as comparisons or JSX elements/fragments.");
        let pos = self.node_pos();
        self.parse_expected(Kind::LessThanToken);
        let type_node = self.parse_type();
        self.parse_expected(Kind::GreaterThanToken);
        let expression = self.parse_simple_unary_expression();
        let node = self.factory.new_type_assertion(type_node, expression);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_left_hand_side_expression_or_higher(&mut self) -> P<Node> {
        // Original Ecma:
        // LeftHandSideExpression: See 11.2
        //      NewExpression
        //      CallExpression
        //
        // Our simplification:
        //
        // LeftHandSideExpression: See 11.2
        //      MemberExpression
        //      CallExpression
        //
        // See comment in parseMemberExpressionOrHigher on how we replaced NewExpression with
        // MemberExpression to make our lives easier.
        //
        // to best understand the below code, it's important to see how CallExpression expands
        // out into its own productions:
        //
        // CallExpression:
        //      MemberExpression Arguments
        //      CallExpression Arguments
        //      CallExpression[Expression]
        //      CallExpression.IdentifierName
        //      import (AssignmentExpression)
        //      super Arguments
        //      super.IdentifierName
        //
        // Because of the recursion in these calls, we need to bottom out first. There are three
        // bottom out states we can run into: 1) We see 'super' which must start either of
        // the last two CallExpression productions. 2) We see 'import' which must start import call.
        // 3)we have a MemberExpression which either completes the LeftHandSideExpression,
        // or starts the beginning of the first four CallExpression productions.
        let pos = self.node_pos();
        let expression: P<Node>;
        if self.token == Kind::ImportKeyword {
            if self.look_ahead(Parser::next_token_is_open_paren_or_less_than) {
                // We don't want to eagerly consume all import keyword as import call expression so we look ahead to find "("
                // For example:
                //      var foo3 = require("subfolder
                //      import * as foo1 from "module-from-node
                // We want this import to be a statement rather than import call expression
                self.source_flags |= NodeFlags::PossiblyContainsDynamicImport;
                expression = self.parse_keyword_expression();
            } else if self.look_ahead(Parser::next_token_is_dot) {
                // This is an 'import.*' metaproperty (i.e. 'import.meta')
                self.next_token(); // advance past the 'import'
                self.next_token(); // advance past the dot
                let name = self.parse_identifier_name();
                let node = self.factory.new_meta_property(Kind::ImportKeyword, name);
                expression = self.finish_node(node, pos);
                if expression.text() == "defer" {
                    if self.token == Kind::OpenParenToken || self.token == Kind::LessThanToken {
                        self.source_flags |= NodeFlags::PossiblyContainsDynamicImport;
                    }
                } else {
                    self.source_flags |= NodeFlags::PossiblyContainsImportMeta;
                }
            } else {
                expression = self.parse_member_expression_or_higher();
            }
        } else if self.token == Kind::SuperKeyword {
            expression = self.parse_super_expression();
        } else {
            expression = self.parse_member_expression_or_higher();
        }
        // Now, we *may* be complete.  However, we might have consumed the start of a
        // CallExpression or OptionalExpression.  As such, we need to consume the rest
        // of it here to be complete.
        self.parse_call_expression_rest(pos, expression)
    }

    pub(crate) fn next_token_is_dot(&mut self) -> bool {
        self.next_token() == Kind::DotToken
    }

    pub(crate) fn parse_super_expression(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let mut expression = self.parse_keyword_expression();
        if self.token == Kind::LessThanToken {
            let start_pos = self.node_pos();
            let type_arguments = self.try_parse_type_arguments_in_expression();
            if type_arguments.is_some() {
                let end = self.node_pos();
                self.parse_error_at(start_pos, end, &diagnostics::X_super_may_not_use_type_arguments, &[]);
                if !self.is_template_start_of_tagged_template() {
                    let node = self.factory.new_expression_with_type_arguments(expression, type_arguments);
                    expression = self.finish_node(node, pos);
                }
            }
        }
        if self.token == Kind::OpenParenToken || self.token == Kind::DotToken || self.token == Kind::OpenBracketToken {
            return expression;
        }
        // If we have seen "super" it must be followed by '(' or '.'.
        // If it wasn't then just try to parse out a '.' and report an error.
        self.parse_error_at_current_token(&diagnostics::X_super_must_be_followed_by_an_argument_list_or_member_access, &[]);
        // private names will never work with `super` (`super.#foo`), but that's a semantic error, not syntactic
        let name = self.parse_right_side_of_dot(true /*allowIdentifierNames*/, true /*allowPrivateIdentifiers*/, true /*allowUnicodeEscapeSequenceInIdentifierName*/);
        let node = self.factory.new_property_access_expression(expression, None /*questionDotToken*/, name, NodeFlags::None);
        self.finish_node(node, pos)
    }

    pub(crate) fn is_template_start_of_tagged_template(&mut self) -> bool {
        self.token == Kind::NoSubstitutionTemplateLiteral || self.token == Kind::TemplateHead
    }

    pub(crate) fn try_parse_type_arguments_in_expression(&mut self) -> Option<P<NodeList>> {
        // TypeArguments must not be parsed in JavaScript files to avoid ambiguity with binary operators.
        // Check the cheap preconditions before saving the parser state: unless the current token is `<`
        // (or `<<`, which reScanLessThanToken would split), there is nothing to speculatively parse and
        // the mark/rewind would be a no-op.
        if self.context_flags.intersects(NodeFlags::JavaScriptFile) || (self.token != Kind::LessThanToken && self.token != Kind::LessThanLessThanToken) {
            return None;
        }
        let state = self.mark();
        if self.re_scan_less_than_token() == Kind::LessThanToken {
            self.next_token();
            let type_arguments = self.parse_delimited_list(ParsingContext::TypeArguments, Parser::parse_type);
            // If it doesn't have the closing `>` then it's definitely not an type argument list.
            if self.re_scan_greater_than_token() == Kind::GreaterThanToken {
                self.next_token();
                // We successfully parsed a type argument list. The next token determines whether we want to
                // treat it as such. If the type argument list is followed by `(` or a template literal, as in
                // `f<number>(42)`, we favor the type argument interpretation even though JavaScript would view
                // it as a relational expression.
                if self.can_follow_type_arguments_in_expression() {
                    return type_arguments;
                }
            }
        }
        self.rewind(state);
        None
    }

    pub(crate) fn can_follow_type_arguments_in_expression(&mut self) -> bool {
        match self.token {
            // These tokens can follow a type argument list in a call expression:
            // foo<x>(
            // foo<T> `...`
            // foo<T> `...${100}...`
            Kind::OpenParenToken | Kind::NoSubstitutionTemplateLiteral | Kind::TemplateHead => return true,
            // A type argument list followed by `<` never makes sense, and a type argument list followed
            // by `>` is ambiguous with a (re-scanned) `>>` operator, so we disqualify both. Also, in
            // this context, `+` and `-` are unary operators, not binary operators.
            Kind::LessThanToken | Kind::GreaterThanToken | Kind::PlusToken | Kind::MinusToken => return false,
            _ => {}
        }
        // We favor the type argument list interpretation when it is immediately followed by
        // a line break, a binary operator, or something that can't start an expression.
        self.has_preceding_line_break() || self.is_binary_operator() || !self.is_start_of_expression()
    }

    pub(crate) fn parse_member_expression_or_higher(&mut self) -> P<Node> {
        // Note: to make our lives simpler, we decompose the NewExpression productions and
        // place ObjectCreationExpression and FunctionExpression into PrimaryExpression.
        // like so:
        //
        //   PrimaryExpression : See 11.1
        //      this
        //      Identifier
        //      Literal
        //      ArrayLiteral
        //      ObjectLiteral
        //      (Expression)
        //      FunctionExpression
        //      new MemberExpression Arguments?
        //
        //   MemberExpression : See 11.2
        //      PrimaryExpression
        //      MemberExpression[Expression]
        //      MemberExpression.IdentifierName
        //
        //   CallExpression : See 11.2
        //      MemberExpression
        //      CallExpression Arguments
        //      CallExpression[Expression]
        //      CallExpression.IdentifierName
        //
        // Technically this is ambiguous.  i.e. CallExpression defines:
        //
        //   CallExpression:
        //      CallExpression Arguments
        //
        // If you see: "new Foo()"
        //
        // Then that could be treated as a single ObjectCreationExpression, or it could be
        // treated as the invocation of "new Foo".  We disambiguate that in code (to match
        // the original grammar) by making sure that if we see an ObjectCreationExpression
        // we always consume arguments if they are there. So we treat "new Foo()" as an
        // object creation only, and not at all as an invocation.  Another way to think
        // about this is that for every "new" that we see, we will consume an argument list if
        // it is there as part of the *associated* object creation node.  Any additional
        // argument lists we see, will become invocation expressions.
        //
        // Because there are no other places in the grammar now that refer to FunctionExpression
        // or ObjectCreationExpression, it is safe to push down into the PrimaryExpression
        // production.
        //
        // Because CallExpression and MemberExpression are left recursive, we need to bottom out
        // of the recursion immediately.  So we parse out a primary expression to start with.
        let pos = self.node_pos();
        let expression = self.parse_primary_expression();
        self.parse_member_expression_rest(pos, expression, true /*allowOptionalChain*/)
    }

    pub(crate) fn parse_member_expression_rest(&mut self, pos: i32, expression: P<Node>, allow_optional_chain: bool) -> P<Node> {
        let mut expression = expression;
        loop {
            let mut question_dot_token: Option<P<Node>> = None;
            let is_property_access;
            if allow_optional_chain && self.is_start_of_optional_property_or_element_access_chain() {
                question_dot_token = Some(self.parse_expected_token(Kind::QuestionDotToken));
                is_property_access = token_is_identifier_or_keyword(self.token);
            } else {
                is_property_access = self.parse_optional(Kind::DotToken);
            }
            if is_property_access {
                expression = self.parse_property_access_expression_rest(pos, expression, question_dot_token);
                continue;
            }
            // when in the [Decorator] context, we do not parse ElementAccess as it could be part of a ComputedPropertyName
            if (question_dot_token.is_some() || !self.in_decorator_context()) && self.parse_optional(Kind::OpenBracketToken) {
                expression = self.parse_element_access_expression_rest(pos, expression, question_dot_token);
                continue;
            }
            if self.is_template_start_of_tagged_template() {
                // Absorb type arguments into TemplateExpression when preceding expression is ExpressionWithTypeArguments
                if question_dot_token.is_none() && ast::is_expression_with_type_arguments(expression) {
                    let original = expression.as_expression_with_type_arguments();
                    expression = self.parse_tagged_template_rest(pos, original.expression, question_dot_token, original.type_arguments);
                    self.unparse_expression_with_type_arguments(Some(original.expression), original.type_arguments, expression);
                } else {
                    expression = self.parse_tagged_template_rest(pos, expression, question_dot_token, None /*typeArguments*/);
                }
                continue;
            }
            if question_dot_token.is_none() {
                if self.token == Kind::ExclamationToken && !self.has_preceding_line_break() {
                    self.next_token();
                    let node = self.factory.new_non_null_expression(expression, NodeFlags::None);
                    let node = self.finish_node(node, pos);
                    expression = self.check_js_syntax(node);
                    continue;
                }
                let type_arguments = self.try_parse_type_arguments_in_expression();
                if type_arguments.is_some() {
                    let node = self.factory.new_expression_with_type_arguments(expression, type_arguments);
                    expression = self.finish_node(node, pos);
                    continue;
                }
            }
            return expression;
        }
    }

    pub(crate) fn is_start_of_optional_property_or_element_access_chain(&mut self) -> bool {
        self.token == Kind::QuestionDotToken && self.look_ahead(Parser::next_token_is_identifier_or_keyword_or_open_bracket_or_template)
    }

    pub(crate) fn next_token_is_identifier_or_keyword_or_open_bracket_or_template(&mut self) -> bool {
        self.next_token();
        token_is_identifier_or_keyword(self.token) || self.token == Kind::OpenBracketToken || self.is_template_start_of_tagged_template()
    }

    pub(crate) fn parse_property_access_expression_rest(&mut self, pos: i32, expression: P<Node>, question_dot_token: Option<P<Node>>) -> P<Node> {
        let name = self.parse_right_side_of_dot(true /*allowIdentifierNames*/, true /*allowPrivateIdentifiers*/, true /*allowUnicodeEscapeSequenceInIdentifierName*/);
        let is_optional_chain = question_dot_token.is_some() || self.try_reparse_optional_chain(expression);
        let property_access = self.factory.new_property_access_expression(expression, question_dot_token, name, if is_optional_chain { NodeFlags::OptionalChain } else { NodeFlags::None });
        if is_optional_chain && ast::is_private_identifier(name) {
            let loc = self.skip_range_trivia(name.loc());
            self.parse_error_at_range(loc, &diagnostics::An_optional_chain_cannot_contain_private_identifiers, &[]);
        }
        if ast::is_expression_with_type_arguments(expression) {
            if let Some(type_arguments) = expression.type_argument_list() {
                let loc = TextRange::new(type_arguments.pos() - 1, scanner::skip_trivia(self.source_text, type_arguments.end()) + 1);
                self.parse_error_at_range(loc, &diagnostics::An_instantiation_expression_cannot_be_followed_by_a_property_access, &[]);
            }
        }
        self.finish_node(property_access, pos)
    }

    pub(crate) fn try_reparse_optional_chain(&mut self, node: P<Node>) -> bool {
        if node.flags().intersects(NodeFlags::OptionalChain) {
            return true;
        }
        // check for an optional chain in a non-null expression
        if ast::is_non_null_expression(node) {
            let mut expr = node.expression().unwrap();
            while ast::is_non_null_expression(expr) && !expr.flags().intersects(NodeFlags::OptionalChain) {
                expr = expr.expression().unwrap();
            }
            if expr.flags().intersects(NodeFlags::OptionalChain) {
                // this is part of an optional chain. Walk down from `node` to `expression` and set the flag.
                let mut node = node;
                while ast::is_non_null_expression(node) {
                    node.set_flags(node.flags() | NodeFlags::OptionalChain);
                    node = node.expression().unwrap();
                }
                return true;
            }
        }
        false
    }

    pub(crate) fn parse_element_access_expression_rest(&mut self, pos: i32, expression: P<Node>, question_dot_token: Option<P<Node>>) -> P<Node> {
        let mut argument_expression = self.create_missing_identifier();
        if self.token == Kind::CloseBracketToken {
            let node_pos = self.node_pos();
            self.parse_error_at(node_pos, node_pos, &diagnostics::An_element_access_expression_should_take_an_argument, &[]);
        } else {
            argument_expression = self.parse_expression_allow_in();
        }
        self.parse_expected(Kind::CloseBracketToken);
        let is_optional_chain = question_dot_token.is_some() || self.try_reparse_optional_chain(expression);
        let node = self.factory.new_element_access_expression(expression, question_dot_token, argument_expression, if is_optional_chain { NodeFlags::OptionalChain } else { NodeFlags::None });
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_call_expression_rest(&mut self, pos: i32, expression: P<Node>) -> P<Node> {
        let mut expression = expression;
        loop {
            expression = self.parse_member_expression_rest(pos, expression, true /*allowOptionalChain*/);
            let mut type_arguments: Option<P<NodeList>> = None;
            let question_dot_token = self.parse_optional_token(Kind::QuestionDotToken);
            if question_dot_token.is_some() {
                type_arguments = self.try_parse_type_arguments_in_expression();
                if self.is_template_start_of_tagged_template() {
                    expression = self.parse_tagged_template_rest(pos, expression, question_dot_token, type_arguments);
                    continue;
                }
            }
            if type_arguments.is_some() || self.token == Kind::OpenParenToken {
                // Absorb type arguments into CallExpression when preceding expression is ExpressionWithTypeArguments
                if question_dot_token.is_none() && expression.kind == Kind::ExpressionWithTypeArguments {
                    type_arguments = expression.type_argument_list();
                    expression = expression.as_expression_with_type_arguments().expression;
                }
                let inner = expression;
                let argument_list = self.parse_argument_list();
                let is_optional_chain = question_dot_token.is_some() || self.try_reparse_optional_chain(expression);
                let node = self.factory.new_call_expression(expression, question_dot_token, type_arguments, argument_list, if is_optional_chain { NodeFlags::OptionalChain } else { NodeFlags::None });
                let node = self.finish_node(node, pos);
                expression = self.check_js_syntax(node);
                self.unparse_expression_with_type_arguments(Some(inner), type_arguments, expression);
                continue;
            }
            if question_dot_token.is_some() {
                // We parsed `?.` but then failed to parse anything, so report a missing identifier here.
                self.parse_error_at_current_token(&diagnostics::Identifier_expected, &[]);
                let name = self.create_missing_identifier();
                let node = self.factory.new_property_access_expression(expression, question_dot_token, name, NodeFlags::OptionalChain);
                expression = self.finish_node(node, pos);
            }
            break;
        }
        expression
    }

    pub(crate) fn parse_argument_list(&mut self) -> P<NodeList> {
        self.parse_expected(Kind::OpenParenToken);
        let result = self.parse_delimited_list(ParsingContext::ArgumentExpressions, Parser::parse_argument_expression).unwrap();
        self.parse_expected(Kind::CloseParenToken);
        result
    }

    pub(crate) fn parse_argument_expression(&mut self) -> P<Node> {
        self.do_in_context(NodeFlags::DisallowInContext | NodeFlags::DecoratorContext, false, Parser::parse_argument_or_array_literal_element)
    }

    pub(crate) fn parse_argument_or_array_literal_element(&mut self) -> P<Node> {
        match self.token {
            Kind::DotDotDotToken => return self.parse_spread_element(),
            Kind::CommaToken => {
                let node = self.factory.new_omitted_expression();
                let pos = self.node_pos();
                return self.finish_node(node, pos);
            }
            _ => {}
        }
        self.parse_assignment_expression_or_higher()
    }

    pub(crate) fn parse_spread_element(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.parse_expected(Kind::DotDotDotToken);
        let expression = self.parse_assignment_expression_or_higher();
        let node = self.factory.new_spread_element(expression);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_tagged_template_rest(&mut self, pos: i32, tag: P<Node>, question_dot_token: Option<P<Node>>, type_arguments: Option<P<NodeList>>) -> P<Node> {
        let template: P<Node>;
        if self.token == Kind::NoSubstitutionTemplateLiteral {
            self.re_scan_template_token(true /*isTaggedTemplate*/);
            template = self.parse_literal_expression();
        } else {
            template = self.parse_template_expression(true /*isTaggedTemplate*/);
        }
        let is_optional_chain = question_dot_token.is_some() || tag.flags().intersects(NodeFlags::OptionalChain);
        let node = self.factory.new_tagged_template_expression(tag, question_dot_token, type_arguments, template, if is_optional_chain { NodeFlags::OptionalChain } else { NodeFlags::None });
        let node = self.finish_node(node, pos);
        self.check_js_syntax(node)
    }

    pub(crate) fn parse_template_expression(&mut self, is_tagged_template: bool) -> P<Node> {
        let pos = self.node_pos();
        let head = self.parse_template_head(is_tagged_template);
        let spans = self.parse_template_spans(is_tagged_template);
        let node = self.factory.new_template_expression(head, spans);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_template_spans(&mut self, is_tagged_template: bool) -> P<NodeList> {
        let pos = self.node_pos();
        let mut list: Vec<P<Node>> = Vec::new();
        loop {
            let span = self.parse_template_span(is_tagged_template);
            list.push(span);
            if span.as_template_span().literal.kind != Kind::TemplateMiddle {
                break;
            }
        }
        let end = self.node_pos();
        self.new_node_list(TextRange::new(pos, end), &list)
    }

    pub(crate) fn parse_template_span(&mut self, is_tagged_template: bool) -> P<Node> {
        let pos = self.node_pos();
        let expression = self.parse_expression_allow_in();
        let literal = self.parse_literal_of_template_span(is_tagged_template);
        let node = self.factory.new_template_span(expression, literal);
        self.finish_node(node, pos)
    }

// @@PARSER3_CONTINUE@@
}

// If true, we should abort parsing an error function.
fn type_has_arrow_function_blocking_parse_error(node: P<Node>) -> bool {
    match node.kind {
        Kind::TypeReference => ast::node_is_missing(Some(node.as_type_reference_node().type_name)),
        Kind::FunctionType | Kind::ConstructorType => {
            is_missing_node_list(Some(node.function_like_data().unwrap().parameters)) || type_has_arrow_function_blocking_parse_error(node.type_node().unwrap())
        }
        Kind::ParenthesizedType => type_has_arrow_function_blocking_parse_error(node.type_node().unwrap()),
        _ => false,
    }
}

// shouldConsumeBinaryOperator reports whether an operator binds before the operator represented by currentPrecedence.
// At equal precedence, only the right-associative exponentiation operator binds first.
fn should_consume_binary_operator(operator: Kind, operator_precedence: OperatorPrecedence, current_precedence: OperatorPrecedence) -> bool {
    if operator_precedence > current_precedence {
        return true;
    }
    operator_precedence == current_precedence && operator == Kind::AsteriskAsteriskToken
}

// @@PARSER3_FREE_FUNCTIONS@@
