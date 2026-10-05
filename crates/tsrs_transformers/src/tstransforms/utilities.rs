use super::*;

/// Go `constantExpression(value any, ...)`: `value` is a string or a `jsnum.Number`; anything else yields nil.
#[derive(Clone, Copy)]
pub(crate) enum ConstantValue<'a> {
    String(&'a str),
    Number(Number),
}

// utilities.go:9
pub(crate) fn constant_expression(value: ConstantValue, factory: &printer::NodeFactory) -> Option<P<Node>> {
    match value {
        ConstantValue::String(value) => Some(factory.new_string_literal(alloc_str(value), TokenFlags::None)),
        ConstantValue::Number(value) => {
            if value.is_inf() {
                if value.0 > 0.0 {
                    return Some(factory.new_identifier("Infinity"));
                }
                return Some(factory.new_prefix_unary_expression(Kind::MinusToken, factory.new_identifier("Infinity")));
            }
            if value.is_nan() {
                return Some(factory.new_identifier("NaN"));
            }
            if value.0 < 0.0 {
                return Some(factory.new_prefix_unary_expression(Kind::MinusToken, constant_expression(ConstantValue::Number(-value), factory).unwrap()));
            }
            Some(factory.new_numeric_literal(alloc_str(&value.string()), TokenFlags::None))
        }
    }
}
