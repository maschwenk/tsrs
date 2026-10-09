mod linter;
mod no_floating_promises;
mod rule;
mod utils;

pub use linter::*;
pub use no_floating_promises::NO_FLOATING_PROMISES;
pub use rule::*;

pub fn rule_by_name(name: &str) -> Option<&'static RuleDefinition> {
    match name {
        "no-floating-promises" => Some(&NO_FLOATING_PROMISES),
        _ => None,
    }
}
