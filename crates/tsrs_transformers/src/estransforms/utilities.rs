use crate::*;

// utilities.go:23
pub(crate) fn create_not_null_condition(emit_context: P<EmitContext>, left: P<Node>, right: P<Node>, invert: bool) -> P<Node> {
    let mut token = Kind::ExclamationEqualsEqualsToken;
    let mut op = Kind::AmpersandAmpersandToken;
    if invert {
        token = Kind::EqualsEqualsEqualsToken;
        op = Kind::BarBarToken;
    }

    let f = &emit_context.get().factory;
    f.new_binary_expression(
        None,
        f.new_binary_expression(None, left, None, f.new_token(token), f.new_keyword_expression(Kind::NullKeyword)),
        None,
        f.new_token(op),
        f.new_binary_expression(None, right, None, f.new_token(token), f.new_void_zero_expression()),
    )
}
