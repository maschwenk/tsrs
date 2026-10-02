use tsrs_ast::Kind;

use super::*;
use crate::lsutil::SemicolonPreference;

macro_rules! preds {
    ($($e:expr),* $(,)?) => {
        vec![$($e as ContextPredicate),*]
    };
}

// rules.go:10
pub(crate) fn get_all_rules() -> Vec<RuleSpec> {
    let mut all_tokens: Vec<Kind> = Vec::with_capacity((Kind::LastToken as usize) - (Kind::FirstToken as usize) + 1);
    for token in (Kind::FirstToken as i16)..=(Kind::LastToken as i16) {
        let token = Kind::from_i16(token);
        if token != Kind::EndOfFile {
            all_tokens.push(token);
        }
    }

    let any_token_except = |tokens: &[Kind]| -> TokenRange {
        let mut new_tokens: Vec<Kind> = Vec::with_capacity(all_tokens.len());
        for &token in &all_tokens {
            if tokens.contains(&token) {
                continue;
            }
            new_tokens.push(token);
        }
        TokenRange { is_specific: false, tokens: new_tokens }
    };

    let any_token = TokenRange { is_specific: false, tokens: all_tokens.clone() };

    // Go's tokenRangeFromEx appends to `allTokens`, whose capacity has exactly one free slot (EndOfFile is skipped
    // above), so both calls write that same slot of the shared backing array: the second call overwrites the first,
    // and anyTokenIncludingMultilineComments ends up as allTokens + EndOfFile. Reproduced here.
    let any_token_including_multiline_comments = token_range_from_ex(&all_tokens, &[Kind::EndOfFile]);
    let any_token_including_eof = token_range_from_ex(&all_tokens, &[Kind::EndOfFile]);
    let keywords = token_range_from_range(Kind::FirstKeyword, Kind::LastKeyword);
    let binary_operators = token_range_from_range(Kind::FirstBinaryOperator, Kind::LastBinaryOperator);
    let binary_keyword_operators: Vec<Kind> =
        vec![Kind::InKeyword, Kind::InstanceOfKeyword, Kind::OfKeyword, Kind::AsKeyword, Kind::IsKeyword, Kind::SatisfiesKeyword];
    let unary_prefix_operators: Vec<Kind> = vec![Kind::PlusPlusToken, Kind::MinusToken, Kind::TildeToken, Kind::ExclamationToken];
    let unary_prefix_expressions: Vec<Kind> = vec![
        Kind::NumericLiteral,
        Kind::BigIntLiteral,
        Kind::Identifier,
        Kind::OpenParenToken,
        Kind::OpenBracketToken,
        Kind::OpenBraceToken,
        Kind::ThisKeyword,
        Kind::NewKeyword,
    ];
    let unary_preincrement_expressions: Vec<Kind> = vec![Kind::Identifier, Kind::OpenParenToken, Kind::ThisKeyword, Kind::NewKeyword];
    let unary_postincrement_expressions: Vec<Kind> = vec![Kind::Identifier, Kind::CloseParenToken, Kind::CloseBracketToken, Kind::NewKeyword];
    let unary_predecrement_expressions: Vec<Kind> = vec![Kind::Identifier, Kind::OpenParenToken, Kind::ThisKeyword, Kind::NewKeyword];
    let unary_postdecrement_expressions: Vec<Kind> = vec![Kind::Identifier, Kind::CloseParenToken, Kind::CloseBracketToken, Kind::NewKeyword];
    let comments: Vec<Kind> = vec![Kind::SingleLineCommentTrivia, Kind::MultiLineCommentTrivia];
    let type_keywords: Vec<Kind> = vec![
        Kind::AnyKeyword,
        Kind::AssertsKeyword,
        Kind::BigIntKeyword,
        Kind::BooleanKeyword,
        Kind::FalseKeyword,
        Kind::InferKeyword,
        Kind::KeyOfKeyword,
        Kind::NeverKeyword,
        Kind::NullKeyword,
        Kind::NumberKeyword,
        Kind::ObjectKeyword,
        Kind::ReadonlyKeyword,
        Kind::StringKeyword,
        Kind::SymbolKeyword,
        Kind::TypeOfKeyword,
        Kind::TrueKeyword,
        Kind::VoidKeyword,
        Kind::UndefinedKeyword,
        Kind::UniqueKeyword,
        Kind::UnknownKeyword,
    ];
    let mut type_names: Vec<Kind> = vec![Kind::Identifier];
    type_names.extend_from_slice(&type_keywords);

    // Place a space before open brace in a function declaration
    // TypeScript: Function can have return types, which can be made of tons of different token kinds
    let function_open_brace_left_token_range = any_token_including_multiline_comments.clone();

    // Place a space before open brace in a TypeScript declaration that has braces as children (class, module, enum, etc)
    let type_script_open_brace_left_token_range = token_range_from(&[
        Kind::Identifier,
        Kind::GreaterThanToken,
        Kind::MultiLineCommentTrivia,
        Kind::ClassKeyword,
        Kind::ExportKeyword,
        Kind::ImportKeyword,
    ]);

    // Place a space before open brace in a control flow construct
    let control_open_brace_left_token_range = token_range_from(&[
        Kind::CloseParenToken,
        Kind::MultiLineCommentTrivia,
        Kind::DoKeyword,
        Kind::TryKeyword,
        Kind::FinallyKeyword,
        Kind::ElseKeyword,
        Kind::CatchKeyword,
    ]);

    let high_priority_common_rules: Vec<RuleSpec> = vec![
        // Leave comments alone
        rule("IgnoreBeforeComment", &any_token, &comments, any_context(), RuleAction::StopProcessingSpaceActions, &[]),
        rule("IgnoreAfterLineComment", Kind::SingleLineCommentTrivia, &any_token, any_context(), RuleAction::StopProcessingSpaceActions, &[]),

        rule("NotSpaceBeforeColon", &any_token, Kind::ColonToken, preds![&is_non_jsx_same_line_token_context, &is_not_binary_op_context, &is_not_type_annotation_context], RuleAction::DeleteSpace, &[]),
        rule("SpaceAfterColon", Kind::ColonToken, &any_token, preds![&is_non_jsx_same_line_token_context, &is_not_binary_op_context, &is_next_token_parent_not_jsx_namespaced_name], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBeforeQuestionMark", &any_token, Kind::QuestionToken, preds![&is_non_jsx_same_line_token_context, &is_not_binary_op_context, &is_not_type_annotation_context], RuleAction::DeleteSpace, &[]),
        // insert space after '?' only when it is used in conditional operator
        rule("SpaceAfterQuestionMarkInConditionalOperator", Kind::QuestionToken, &any_token, preds![&is_non_jsx_same_line_token_context, &is_conditional_operator_context], RuleAction::InsertSpace, &[]),

        // in other cases there should be no space between '?' and next token
        rule("NoSpaceAfterQuestionMark", Kind::QuestionToken, &any_token, preds![&is_non_jsx_same_line_token_context, &is_non_optional_property_context], RuleAction::DeleteSpace, &[]),

        rule("NoSpaceBeforeDot", &any_token, [Kind::DotToken, Kind::QuestionDotToken], preds![&is_non_jsx_same_line_token_context, &is_not_property_access_on_integer_literal], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterDot", [Kind::DotToken, Kind::QuestionDotToken], &any_token, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        rule("NoSpaceBetweenImportParenInImportType", Kind::ImportKeyword, Kind::OpenParenToken, preds![&is_non_jsx_same_line_token_context, &is_import_type_context], RuleAction::DeleteSpace, &[]),

        // Special handling of unary operators.
        // Prefix operators generally shouldn't have a space between
        // them and their target unary expression.
        rule("NoSpaceAfterUnaryPrefixOperator", &unary_prefix_operators, &unary_prefix_expressions, preds![&is_non_jsx_same_line_token_context, &is_not_binary_op_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterUnaryPreincrementOperator", Kind::PlusPlusToken, &unary_preincrement_expressions, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterUnaryPredecrementOperator", Kind::MinusMinusToken, &unary_predecrement_expressions, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceBeforeUnaryPostincrementOperator", &unary_postincrement_expressions, Kind::PlusPlusToken, preds![&is_non_jsx_same_line_token_context, &is_not_statement_condition_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceBeforeUnaryPostdecrementOperator", &unary_postdecrement_expressions, Kind::MinusMinusToken, preds![&is_non_jsx_same_line_token_context, &is_not_statement_condition_context], RuleAction::DeleteSpace, &[]),

        // More unary operator special-casing.
        // DevDiv 181814: Be careful when removing leading whitespace
        // around unary operators.  Examples:
        //      1 - -2  --X--> 1--2
        //      a + ++b --X--> a+++b
        rule("SpaceAfterPostincrementWhenFollowedByAdd", Kind::PlusPlusToken, Kind::PlusToken, preds![&is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::InsertSpace, &[]),
        rule("SpaceAfterAddWhenFollowedByUnaryPlus", Kind::PlusToken, Kind::PlusToken, preds![&is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::InsertSpace, &[]),
        rule("SpaceAfterAddWhenFollowedByPreincrement", Kind::PlusToken, Kind::PlusPlusToken, preds![&is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::InsertSpace, &[]),
        rule("SpaceAfterPostdecrementWhenFollowedBySubtract", Kind::MinusMinusToken, Kind::MinusToken, preds![&is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::InsertSpace, &[]),
        rule("SpaceAfterSubtractWhenFollowedByUnaryMinus", Kind::MinusToken, Kind::MinusToken, preds![&is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::InsertSpace, &[]),
        rule("SpaceAfterSubtractWhenFollowedByPredecrement", Kind::MinusToken, Kind::MinusMinusToken, preds![&is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::InsertSpace, &[]),

        rule("NoSpaceAfterCloseBrace", Kind::CloseBraceToken, [Kind::CommaToken, Kind::SemicolonToken], preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        // For functions and control block place } on a new line []ast.Kind{multi-line rule}
        rule("NewLineBeforeCloseBraceInBlockContext", &any_token_including_multiline_comments, Kind::CloseBraceToken, preds![&is_multiline_block_context], RuleAction::InsertNewLine, &[]),

        // Space/new line after }.
        rule("SpaceAfterCloseBrace", Kind::CloseBraceToken, &any_token_except(&[Kind::CloseParenToken]), preds![&is_non_jsx_same_line_token_context, &is_after_code_block_context], RuleAction::InsertSpace, &[]),
        // Special case for (}, else) and (}, while) since else & while tokens are not part of the tree which makes SpaceAfterCloseBrace rule not applied
        // Also should not apply to })
        rule("SpaceBetweenCloseBraceAndElse", Kind::CloseBraceToken, Kind::ElseKeyword, preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("SpaceBetweenCloseBraceAndWhile", Kind::CloseBraceToken, Kind::WhileKeyword, preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBetweenEmptyBraceBrackets", Kind::OpenBraceToken, Kind::CloseBraceToken, preds![&is_non_jsx_same_line_token_context, &is_object_context], RuleAction::DeleteSpace, &[]),

        // Add a space after control dec context if the next character is an open bracket ex: 'if (false)[]ast.Kind{a, b} = []ast.Kind{1, 2};' -> 'if (false) []ast.Kind{a, b} = []ast.Kind{1, 2};'
        rule("SpaceAfterConditionalClosingParen", Kind::CloseParenToken, Kind::OpenBracketToken, preds![&is_control_decl_context], RuleAction::InsertSpace, &[]),

        rule("NoSpaceBetweenFunctionKeywordAndStar", Kind::FunctionKeyword, Kind::AsteriskToken, preds![&is_function_declaration_or_function_expression_context], RuleAction::DeleteSpace, &[]),
        rule("SpaceAfterStarInGeneratorDeclaration", Kind::AsteriskToken, Kind::Identifier, preds![&is_function_declaration_or_function_expression_context], RuleAction::InsertSpace, &[]),

        rule("SpaceAfterFunctionInFuncDecl", Kind::FunctionKeyword, &any_token, preds![&is_function_decl_context], RuleAction::InsertSpace, &[]),
        // Insert new line after { and before } in multi-line contexts.
        rule("NewLineAfterOpenBraceInBlockContext", Kind::OpenBraceToken, &any_token, preds![&is_multiline_block_context], RuleAction::InsertNewLine, &[]),

        // For get/set members, we check for (identifier,identifier) since get/set don't have tokens and they are represented as just an identifier token.
        // Though, we do extra check on the context to make sure we are dealing with get/set node. Example:
        //      get x() {}
        //      set x(val) {}
        rule("SpaceAfterGetSetInMember", [Kind::GetKeyword, Kind::SetKeyword], Kind::Identifier, preds![&is_function_decl_context], RuleAction::InsertSpace, &[]),

        rule("NoSpaceBetweenYieldKeywordAndStar", Kind::YieldKeyword, Kind::AsteriskToken, preds![&is_non_jsx_same_line_token_context, &is_yield_or_yield_star_with_operand], RuleAction::DeleteSpace, &[]),
        rule("SpaceBetweenYieldOrYieldStarAndOperand", [Kind::YieldKeyword, Kind::AsteriskToken], &any_token, preds![&is_non_jsx_same_line_token_context, &is_yield_or_yield_star_with_operand], RuleAction::InsertSpace, &[]),

        rule("NoSpaceBetweenReturnAndSemicolon", Kind::ReturnKeyword, Kind::SemicolonToken, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("SpaceAfterCertainKeywords", [Kind::VarKeyword, Kind::ThrowKeyword, Kind::NewKeyword, Kind::DeleteKeyword, Kind::ReturnKeyword, Kind::TypeOfKeyword, Kind::AwaitKeyword], &any_token, preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("SpaceAfterLetConstInVariableDeclaration", [Kind::LetKeyword, Kind::ConstKeyword], &any_token, preds![&is_non_jsx_same_line_token_context, &is_start_of_variable_declaration_list], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBeforeOpenParenInFuncCall", &any_token, Kind::OpenParenToken, preds![&is_non_jsx_same_line_token_context, &is_function_call_or_new_context, &is_previous_token_not_comma], RuleAction::DeleteSpace, &[]),

        // Special case for binary operators (that are keywords). For these we have to add a space and shouldn't follow any user options.
        rule("SpaceBeforeBinaryKeywordOperator", &any_token, &binary_keyword_operators, preds![&is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::InsertSpace, &[]),
        rule("SpaceAfterBinaryKeywordOperator", &binary_keyword_operators, &any_token, preds![&is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::InsertSpace, &[]),

        rule("SpaceAfterVoidOperator", Kind::VoidKeyword, &any_token, preds![&is_non_jsx_same_line_token_context, &is_void_op_context], RuleAction::InsertSpace, &[]),

        // Async-await
        rule("SpaceBetweenAsyncAndOpenParen", Kind::AsyncKeyword, Kind::OpenParenToken, preds![&is_arrow_function_context, &is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("SpaceBetweenAsyncAndFunctionKeyword", Kind::AsyncKeyword, [Kind::FunctionKeyword, Kind::Identifier], preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),

        // Template string
        rule("NoSpaceBetweenTagAndTemplateString", [Kind::Identifier, Kind::CloseParenToken], [Kind::NoSubstitutionTemplateLiteral, Kind::TemplateHead], preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        // JSX opening elements
        rule("SpaceBeforeJsxAttribute", &any_token, Kind::Identifier, preds![&is_next_token_parent_jsx_attribute, &is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("SpaceBeforeSlashInJsxOpeningElement", &any_token, Kind::SlashToken, preds![&is_jsx_self_closing_element_context, &is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBeforeGreaterThanTokenInJsxOpeningElement", Kind::SlashToken, Kind::GreaterThanToken, preds![&is_jsx_self_closing_element_context, &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceBeforeEqualInJsxAttribute", &any_token, Kind::EqualsToken, preds![&is_jsx_attribute_context, &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterEqualInJsxAttribute", Kind::EqualsToken, &any_token, preds![&is_jsx_attribute_context, &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceBeforeJsxNamespaceColon", Kind::Identifier, Kind::ColonToken, preds![&is_next_token_parent_jsx_namespaced_name], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterJsxNamespaceColon", Kind::ColonToken, Kind::Identifier, preds![&is_next_token_parent_jsx_namespaced_name], RuleAction::DeleteSpace, &[]),

        // TypeScript-specific rules
        // Use of module as a function call. e.g.: import m2 = module("m2");
        rule("NoSpaceAfterModuleImport", [Kind::ModuleKeyword, Kind::RequireKeyword], Kind::OpenParenToken, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        // Add a space around certain TypeScript keywords
        rule(
            "SpaceAfterCertainTypeScriptKeywords",
            [
                Kind::AbstractKeyword,
                Kind::AccessorKeyword,
                Kind::ClassKeyword,
                Kind::DeclareKeyword,
                Kind::DefaultKeyword,
                Kind::EnumKeyword,
                Kind::ExportKeyword,
                Kind::ExtendsKeyword,
                Kind::GetKeyword,
                Kind::ImplementsKeyword,
                Kind::ImportKeyword,
                Kind::InterfaceKeyword,
                Kind::ModuleKeyword,
                Kind::NamespaceKeyword,
                Kind::OverrideKeyword,
                Kind::PrivateKeyword,
                Kind::PublicKeyword,
                Kind::ProtectedKeyword,
                Kind::ReadonlyKeyword,
                Kind::SetKeyword,
                Kind::StaticKeyword,
                Kind::TypeKeyword,
                Kind::FromKeyword,
                Kind::KeyOfKeyword,
                Kind::InferKeyword,
            ],
            &any_token,
            preds![&is_non_jsx_same_line_token_context],
            RuleAction::InsertSpace, &[],
        ),
        rule(
            "SpaceBeforeCertainTypeScriptKeywords",
            &any_token,
            [Kind::ExtendsKeyword, Kind::ImplementsKeyword, Kind::FromKeyword],
            preds![&is_non_jsx_same_line_token_context],
            RuleAction::InsertSpace, &[],
        ),
        // Treat string literals in module names as identifiers, and add a space between the literal and the opening Brace braces, e.g.: module "m2" {
        rule("SpaceAfterModuleName", Kind::StringLiteral, Kind::OpenBraceToken, preds![&is_module_decl_context], RuleAction::InsertSpace, &[]),

        // Lambda expressions
        rule("SpaceBeforeArrow", &any_token, Kind::EqualsGreaterThanToken, preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("SpaceAfterArrow", Kind::EqualsGreaterThanToken, &any_token, preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),

        // Optional parameters and let args
        rule("NoSpaceAfterEllipsis", Kind::DotDotDotToken, Kind::Identifier, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterOptionalParameters", Kind::QuestionToken, [Kind::CloseParenToken, Kind::CommaToken], preds![&is_non_jsx_same_line_token_context, &is_not_binary_op_context], RuleAction::DeleteSpace, &[]),

        // Remove spaces in empty interface literals. e.g.: x: {}
        rule("NoSpaceBetweenEmptyInterfaceBraceBrackets", Kind::OpenBraceToken, Kind::CloseBraceToken, preds![&is_non_jsx_same_line_token_context, &is_object_type_context], RuleAction::DeleteSpace, &[]),

        // generics and type assertions
        rule("NoSpaceBeforeOpenAngularBracket", &type_names, Kind::LessThanToken, preds![&is_non_jsx_same_line_token_context, &is_type_argument_or_parameter_or_assertion_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceBetweenCloseParenAndAngularBracket", Kind::CloseParenToken, Kind::LessThanToken, preds![&is_non_jsx_same_line_token_context, &is_type_argument_or_parameter_or_assertion_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterOpenAngularBracket", Kind::LessThanToken, &any_token, preds![&is_non_jsx_same_line_token_context, &is_type_argument_or_parameter_or_assertion_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceBeforeCloseAngularBracket", &any_token, Kind::GreaterThanToken, preds![&is_non_jsx_same_line_token_context, &is_type_argument_or_parameter_or_assertion_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterCloseAngularBracket", Kind::GreaterThanToken, [Kind::OpenParenToken, Kind::OpenBracketToken, Kind::GreaterThanToken, Kind::CommaToken], preds![
            &is_non_jsx_same_line_token_context,
            &is_type_argument_or_parameter_or_assertion_context,
            &is_not_function_decl_context, /*To prevent an interference with the SpaceBeforeOpenParenInFuncDecl rule*/
            &is_non_type_assertion_context,
        ], RuleAction::DeleteSpace, &[]),

        // decorators
        rule("SpaceBeforeAt", [Kind::CloseParenToken, Kind::Identifier], Kind::AtToken, preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceAfterAt", Kind::AtToken, &any_token, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        // Insert space after @ in decorator
        rule(
            "SpaceAfterDecorator",
            &any_token,
            [
                Kind::AbstractKeyword,
                Kind::Identifier,
                Kind::ExportKeyword,
                Kind::DefaultKeyword,
                Kind::ClassKeyword,
                Kind::StaticKeyword,
                Kind::PublicKeyword,
                Kind::PrivateKeyword,
                Kind::ProtectedKeyword,
                Kind::GetKeyword,
                Kind::SetKeyword,
                Kind::OpenBracketToken,
                Kind::AsteriskToken,
            ],
            preds![&is_end_of_decorator_context_on_same_line],
            RuleAction::InsertSpace, &[],
        ),

        rule("NoSpaceBeforeNonNullAssertionOperator", &any_token, Kind::ExclamationToken, preds![&is_non_jsx_same_line_token_context, &is_non_null_assertion_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterNewKeywordOnConstructorSignature", Kind::NewKeyword, Kind::OpenParenToken, preds![&is_non_jsx_same_line_token_context, &is_constructor_signature_context], RuleAction::DeleteSpace, &[]),
        rule("SpaceLessThanAndNonJSXTypeAnnotation", Kind::LessThanToken, Kind::LessThanToken, preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
    ];

    // These rules are applied after high priority
    let user_configurable_rules: Vec<RuleSpec> = vec![
        // Treat constructor as an identifier in a function declaration, and remove spaces between constructor and following left parentheses
        rule("SpaceAfterConstructor", Kind::ConstructorKeyword, Kind::OpenParenToken, preds![is_option_enabled(insert_space_after_constructor_option), &is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceAfterConstructor", Kind::ConstructorKeyword, Kind::OpenParenToken, preds![is_option_disabled_or_undefined(insert_space_after_constructor_option), &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        rule("SpaceAfterComma", Kind::CommaToken, &any_token, preds![is_option_enabled(insert_space_after_comma_delimiter_option), &is_non_jsx_same_line_token_context, &is_non_jsx_element_or_fragment_context, &is_next_token_not_close_bracket, &is_next_token_not_close_paren], RuleAction::InsertSpace, &[]),
        rule("NoSpaceAfterComma", Kind::CommaToken, &any_token, preds![is_option_disabled_or_undefined(insert_space_after_comma_delimiter_option), &is_non_jsx_same_line_token_context, &is_non_jsx_element_or_fragment_context], RuleAction::DeleteSpace, &[]),

        // Insert space after function keyword for anonymous functions
        rule("SpaceAfterAnonymousFunctionKeyword", [Kind::FunctionKeyword, Kind::AsteriskToken], Kind::OpenParenToken, preds![is_option_enabled(insert_space_after_function_keyword_for_anonymous_functions_option), &is_function_decl_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceAfterAnonymousFunctionKeyword", [Kind::FunctionKeyword, Kind::AsteriskToken], Kind::OpenParenToken, preds![is_option_disabled_or_undefined(insert_space_after_function_keyword_for_anonymous_functions_option), &is_function_decl_context], RuleAction::DeleteSpace, &[]),

        // Insert space after keywords in control flow statements
        rule("SpaceAfterKeywordInControl", &keywords, Kind::OpenParenToken, preds![is_option_enabled(insert_space_after_keywords_in_control_flow_statements_option), &is_control_decl_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceAfterKeywordInControl", &keywords, Kind::OpenParenToken, preds![is_option_disabled_or_undefined(insert_space_after_keywords_in_control_flow_statements_option), &is_control_decl_context], RuleAction::DeleteSpace, &[]),

        // Insert space after opening and before closing nonempty parenthesis
        rule("SpaceAfterOpenParen", Kind::OpenParenToken, &any_token, preds![is_option_enabled(insert_space_after_opening_and_before_closing_nonempty_parenthesis_option), &is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("SpaceBeforeCloseParen", &any_token, Kind::CloseParenToken, preds![is_option_enabled(insert_space_after_opening_and_before_closing_nonempty_parenthesis_option), &is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("SpaceBetweenOpenParens", Kind::OpenParenToken, Kind::OpenParenToken, preds![is_option_enabled(insert_space_after_opening_and_before_closing_nonempty_parenthesis_option), &is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBetweenParens", Kind::OpenParenToken, Kind::CloseParenToken, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterOpenParen", Kind::OpenParenToken, &any_token, preds![is_option_disabled_or_undefined(insert_space_after_opening_and_before_closing_nonempty_parenthesis_option), &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceBeforeCloseParen", &any_token, Kind::CloseParenToken, preds![is_option_disabled_or_undefined(insert_space_after_opening_and_before_closing_nonempty_parenthesis_option), &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        // Insert space after opening and before closing nonempty brackets
        rule("SpaceAfterOpenBracket", Kind::OpenBracketToken, &any_token, preds![is_option_enabled(insert_space_after_opening_and_before_closing_nonempty_brackets_option), &is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("SpaceBeforeCloseBracket", &any_token, Kind::CloseBracketToken, preds![is_option_enabled(insert_space_after_opening_and_before_closing_nonempty_brackets_option), &is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBetweenBrackets", Kind::OpenBracketToken, Kind::CloseBracketToken, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterOpenBracket", Kind::OpenBracketToken, &any_token, preds![is_option_disabled_or_undefined(insert_space_after_opening_and_before_closing_nonempty_brackets_option), &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceBeforeCloseBracket", &any_token, Kind::CloseBracketToken, preds![is_option_disabled_or_undefined(insert_space_after_opening_and_before_closing_nonempty_brackets_option), &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        // Insert a space after { and before } in single-line contexts, but remove space from empty object literals {}.
        rule("SpaceAfterOpenBrace", Kind::OpenBraceToken, &any_token, preds![is_option_enabled_or_undefined(insert_space_after_opening_and_before_closing_nonempty_braces_option), &is_brace_wrapped_context], RuleAction::InsertSpace, &[]),
        rule("SpaceBeforeCloseBrace", &any_token, Kind::CloseBraceToken, preds![is_option_enabled_or_undefined(insert_space_after_opening_and_before_closing_nonempty_braces_option), &is_brace_wrapped_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBetweenEmptyBraceBrackets", Kind::OpenBraceToken, Kind::CloseBraceToken, preds![&is_non_jsx_same_line_token_context, &is_object_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterOpenBrace", Kind::OpenBraceToken, &any_token, preds![is_option_disabled(insert_space_after_opening_and_before_closing_nonempty_braces_option), &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceBeforeCloseBrace", &any_token, Kind::CloseBraceToken, preds![is_option_disabled(insert_space_after_opening_and_before_closing_nonempty_braces_option), &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        // Insert a space after opening and before closing empty brace brackets
        rule("SpaceBetweenEmptyBraceBrackets", Kind::OpenBraceToken, Kind::CloseBraceToken, preds![is_option_enabled(insert_space_after_opening_and_before_closing_empty_braces_option)], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBetweenEmptyBraceBrackets", Kind::OpenBraceToken, Kind::CloseBraceToken, preds![is_option_disabled(insert_space_after_opening_and_before_closing_empty_braces_option), &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        // Insert space after opening and before closing template string braces
        rule("SpaceAfterTemplateHeadAndMiddle", [Kind::TemplateHead, Kind::TemplateMiddle], &any_token, preds![is_option_enabled(insert_space_after_opening_and_before_closing_template_string_braces_option), &is_non_jsx_text_context], RuleAction::InsertSpace, &[RuleFlags::CanDeleteNewLines]),
        rule("SpaceBeforeTemplateMiddleAndTail", &any_token, [Kind::TemplateMiddle, Kind::TemplateTail], preds![is_option_enabled(insert_space_after_opening_and_before_closing_template_string_braces_option), &is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceAfterTemplateHeadAndMiddle", [Kind::TemplateHead, Kind::TemplateMiddle], &any_token, preds![is_option_disabled_or_undefined(insert_space_after_opening_and_before_closing_template_string_braces_option), &is_non_jsx_text_context], RuleAction::DeleteSpace, &[RuleFlags::CanDeleteNewLines]),
        rule("NoSpaceBeforeTemplateMiddleAndTail", &any_token, [Kind::TemplateMiddle, Kind::TemplateTail], preds![is_option_disabled_or_undefined(insert_space_after_opening_and_before_closing_template_string_braces_option), &is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        // No space after { and before } in JSX expression
        rule("SpaceAfterOpenBraceInJsxExpression", Kind::OpenBraceToken, &any_token, preds![is_option_enabled(insert_space_after_opening_and_before_closing_jsx_expression_braces_option), &is_non_jsx_same_line_token_context, &is_jsx_expression_context], RuleAction::InsertSpace, &[]),
        rule("SpaceBeforeCloseBraceInJsxExpression", &any_token, Kind::CloseBraceToken, preds![is_option_enabled(insert_space_after_opening_and_before_closing_jsx_expression_braces_option), &is_non_jsx_same_line_token_context, &is_jsx_expression_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceAfterOpenBraceInJsxExpression", Kind::OpenBraceToken, &any_token, preds![is_option_disabled_or_undefined(insert_space_after_opening_and_before_closing_jsx_expression_braces_option), &is_non_jsx_same_line_token_context, &is_jsx_expression_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceBeforeCloseBraceInJsxExpression", &any_token, Kind::CloseBraceToken, preds![is_option_disabled_or_undefined(insert_space_after_opening_and_before_closing_jsx_expression_braces_option), &is_non_jsx_same_line_token_context, &is_jsx_expression_context], RuleAction::DeleteSpace, &[]),

        // Insert space after semicolon in for statement
        rule("SpaceAfterSemicolonInFor", Kind::SemicolonToken, &any_token, preds![is_option_enabled(insert_space_after_semicolon_in_for_statements_option), &is_non_jsx_same_line_token_context, &is_for_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceAfterSemicolonInFor", Kind::SemicolonToken, &any_token, preds![is_option_disabled_or_undefined(insert_space_after_semicolon_in_for_statements_option), &is_non_jsx_same_line_token_context, &is_for_context], RuleAction::DeleteSpace, &[]),

        // Insert space before and after binary operators
        rule("SpaceBeforeBinaryOperator", &any_token, &binary_operators, preds![is_option_enabled(insert_space_before_and_after_binary_operators_option), &is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::InsertSpace, &[]),
        rule("SpaceAfterBinaryOperator", &binary_operators, &any_token, preds![is_option_enabled(insert_space_before_and_after_binary_operators_option), &is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBeforeBinaryOperator", &any_token, &binary_operators, preds![is_option_disabled_or_undefined(insert_space_before_and_after_binary_operators_option), &is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterBinaryOperator", &binary_operators, &any_token, preds![is_option_disabled_or_undefined(insert_space_before_and_after_binary_operators_option), &is_non_jsx_same_line_token_context, &is_binary_op_context], RuleAction::DeleteSpace, &[]),

        rule("SpaceBeforeOpenParenInFuncDecl", &any_token, Kind::OpenParenToken, preds![is_option_enabled(insert_space_before_function_parenthesis_option), &is_non_jsx_same_line_token_context, &is_function_decl_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBeforeOpenParenInFuncDecl", &any_token, Kind::OpenParenToken, preds![is_option_disabled_or_undefined(insert_space_before_function_parenthesis_option), &is_non_jsx_same_line_token_context, &is_function_decl_context], RuleAction::DeleteSpace, &[]),

        // Open Brace braces after control block
        rule("NewLineBeforeOpenBraceInControl", &control_open_brace_left_token_range, Kind::OpenBraceToken, preds![is_option_enabled(place_open_brace_on_new_line_for_control_blocks_option), &is_control_decl_context, &is_before_multiline_block_context], RuleAction::InsertNewLine, &[RuleFlags::CanDeleteNewLines]),

        // Open Brace braces after function
        // TypeScript: Function can have return types, which can be made of tons of different token kinds
        rule("NewLineBeforeOpenBraceInFunction", &function_open_brace_left_token_range, Kind::OpenBraceToken, preds![is_option_enabled(place_open_brace_on_new_line_for_functions_option), &is_function_decl_context, &is_before_multiline_block_context], RuleAction::InsertNewLine, &[RuleFlags::CanDeleteNewLines]),
        // Open Brace braces after TypeScript module/class/interface
        rule("NewLineBeforeOpenBraceInTypeScriptDeclWithBlock", &type_script_open_brace_left_token_range, Kind::OpenBraceToken, preds![is_option_enabled(place_open_brace_on_new_line_for_functions_option), &is_type_script_decl_with_block_context, &is_before_multiline_block_context], RuleAction::InsertNewLine, &[RuleFlags::CanDeleteNewLines]),

        rule("SpaceAfterTypeAssertion", Kind::GreaterThanToken, &any_token, preds![is_option_enabled(insert_space_after_type_assertion_option), &is_non_jsx_same_line_token_context, &is_type_assertion_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceAfterTypeAssertion", Kind::GreaterThanToken, &any_token, preds![is_option_disabled_or_undefined(insert_space_after_type_assertion_option), &is_non_jsx_same_line_token_context, &is_type_assertion_context], RuleAction::DeleteSpace, &[]),

        rule("SpaceBeforeTypeAnnotation", &any_token, [Kind::QuestionToken, Kind::ColonToken], preds![is_option_enabled(insert_space_before_type_annotation_option), &is_non_jsx_same_line_token_context, &is_type_annotation_context], RuleAction::InsertSpace, &[]),
        rule("NoSpaceBeforeTypeAnnotation", &any_token, [Kind::QuestionToken, Kind::ColonToken], preds![is_option_disabled_or_undefined(insert_space_before_type_annotation_option), &is_non_jsx_same_line_token_context, &is_type_annotation_context], RuleAction::DeleteSpace, &[]),

        rule("NoOptionalSemicolon", Kind::SemicolonToken, &any_token_including_eof, preds![option_equals(semicolon_option, SemicolonPreference::Remove), &is_semicolon_deletion_context], RuleAction::DeleteToken, &[]),
        rule("OptionalSemicolon", &any_token, &any_token_including_eof, preds![option_equals(semicolon_option, SemicolonPreference::Insert), &is_semicolon_insertion_context], RuleAction::InsertTrailingSemicolon, &[]),
    ];

    // These rules are lower in priority than user-configurable. Rules earlier in this list have priority over rules later in the list.
    let low_priority_common_rules: Vec<RuleSpec> = vec![
        // Space after keyword but not before ; or : or ?
        rule("NoSpaceBeforeSemicolon", &any_token, Kind::SemicolonToken, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        rule("SpaceBeforeOpenBraceInControl", &control_open_brace_left_token_range, Kind::OpenBraceToken, preds![is_option_disabled_or_undefined_or_tokens_on_same_line(place_open_brace_on_new_line_for_control_blocks_option), &is_control_decl_context, &is_not_format_on_enter, &is_same_line_token_or_before_block_context], RuleAction::InsertSpace, &[RuleFlags::CanDeleteNewLines]),
        rule("SpaceBeforeOpenBraceInFunction", &function_open_brace_left_token_range, Kind::OpenBraceToken, preds![is_option_disabled_or_undefined_or_tokens_on_same_line(place_open_brace_on_new_line_for_functions_option), &is_function_decl_context, &is_before_block_context, &is_not_format_on_enter, &is_same_line_token_or_before_block_context], RuleAction::InsertSpace, &[RuleFlags::CanDeleteNewLines]),
        rule("SpaceBeforeOpenBraceInTypeScriptDeclWithBlock", &type_script_open_brace_left_token_range, Kind::OpenBraceToken, preds![is_option_disabled_or_undefined_or_tokens_on_same_line(place_open_brace_on_new_line_for_functions_option), &is_type_script_decl_with_block_context, &is_not_format_on_enter, &is_same_line_token_or_before_block_context], RuleAction::InsertSpace, &[RuleFlags::CanDeleteNewLines]),

        rule("NoSpaceBeforeComma", &any_token, Kind::CommaToken, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        // No space before and after indexer `x[]ast.Kind{}`
        rule("NoSpaceBeforeOpenBracket", &any_token_except(&[Kind::AsyncKeyword, Kind::CaseKeyword]), Kind::OpenBracketToken, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),
        rule("NoSpaceAfterCloseBracket", Kind::CloseBracketToken, &any_token, preds![&is_non_jsx_same_line_token_context, &is_not_before_block_in_function_declaration_context], RuleAction::DeleteSpace, &[]),
        rule("SpaceAfterSemicolon", Kind::SemicolonToken, &any_token, preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),

        // Remove extra space between for and await
        rule("SpaceBetweenForAndAwaitKeyword", Kind::ForKeyword, Kind::AwaitKeyword, preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),

        // Remove extra spaces between ... and type name in tuple spread
        rule("SpaceBetweenDotDotDotAndTypeName", Kind::DotDotDotToken, &type_names, preds![&is_non_jsx_same_line_token_context], RuleAction::DeleteSpace, &[]),

        // Add a space between statements. All keywords except (do,else,case) has open/close parens after them.
        // So, we have a rule to add a space for []ast.Kind{),Any}, []ast.Kind{do,Any}, []ast.Kind{else,Any}, and []ast.Kind{case,Any}
        rule(
            "SpaceBetweenStatements",
            [Kind::CloseParenToken, Kind::DoKeyword, Kind::ElseKeyword, Kind::CaseKeyword],
            &any_token,
            preds![&is_non_jsx_same_line_token_context, &is_non_jsx_element_or_fragment_context, &is_not_for_context],
            RuleAction::InsertSpace, &[],
        ),
        // This low-pri rule takes care of "try {", "catch {" and "finally {" in case the rule SpaceBeforeOpenBraceInControl didn't execute on FormatOnEnter.
        rule("SpaceAfterTryCatchFinally", [Kind::TryKeyword, Kind::CatchKeyword, Kind::FinallyKeyword], Kind::OpenBraceToken, preds![&is_non_jsx_same_line_token_context], RuleAction::InsertSpace, &[]),
    ];

    let mut result: Vec<RuleSpec> =
        Vec::with_capacity(high_priority_common_rules.len() + user_configurable_rules.len() + low_priority_common_rules.len());
    result.extend(high_priority_common_rules);
    result.extend(user_configurable_rules);
    result.extend(low_priority_common_rules);
    result
}

// rules.go:428
fn token_range_from(tokens: &[Kind]) -> TokenRange {
    TokenRange { is_specific: true, tokens: tokens.to_vec() }
}

// rules.go:435
fn token_range_from_ex(prefix: &[Kind], tokens: &[Kind]) -> TokenRange {
    let mut all = prefix.to_vec();
    all.extend_from_slice(tokens);
    TokenRange { is_specific: true, tokens: all }
}

// rules.go:443
fn token_range_from_range(start: Kind, end: Kind) -> TokenRange {
    let mut tokens: Vec<Kind> = Vec::with_capacity((end as usize) - (start as usize) + 1);
    for token in (start as i16)..=(end as i16) {
        tokens.push(Kind::from_i16(token));
    }

    token_range_from(&tokens)
}
