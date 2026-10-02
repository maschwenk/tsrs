// PARTIAL port of completions.go (completions are phase 3): only what the phase-1 files use.

use tsrs_ast::{Kind, Node};
use tsrs_checker::{Checker, Type};
use tsrs_core::P;
use tsrs_lsproto as lsproto;

// completions.go:35 (Go compares with `errors.Is`; `is_err_needs_auto_imports` compares the message)
pub const ERR_NEEDS_AUTO_IMPORTS: &str = "completion list needs auto imports";

pub fn err_needs_auto_imports() -> lsproto::Error {
    lsproto::Error::new(ERR_NEEDS_AUTO_IMPORTS)
}

pub fn is_err_needs_auto_imports(err: &lsproto::Error) -> bool {
    err.message == ERR_NEEDS_AUTO_IMPORTS
}

// completions.go:3557
pub(crate) fn get_switched_type(case_clause: P<Node>, type_checker: &mut Checker) -> P<Type> {
    type_checker.get_type_at_location(case_clause.parent().unwrap().parent().unwrap().expression().unwrap())
}

// completions.go:3561
pub(crate) fn is_equality_operator_kind(kind: Kind) -> bool {
    matches!(kind, Kind::EqualsEqualsEqualsToken | Kind::EqualsEqualsToken | Kind::ExclamationEqualsEqualsToken | Kind::ExclamationEqualsToken)
}
