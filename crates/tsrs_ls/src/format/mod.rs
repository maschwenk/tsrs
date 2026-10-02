mod api;
mod context;
mod indent;
mod rule;
mod rulecontext;
mod rules;
mod rulesmap;
mod scanner;
mod span;
mod util;
pub use api::*;
pub use context::*;
pub use indent::*;
pub use rule::*;
pub use rulecontext::*;
pub use rules::*;
pub use rulesmap::*;
pub use scanner::*;
pub use span::*;
pub use util::*;

#[cfg(test)]
mod api_test;
#[cfg(test)]
mod comment_test;
#[cfg(test)]
mod format_test;
#[cfg(test)]
mod indent_getindentation_test;
#[cfg(test)]
mod indent_test;
#[cfg(test)]
mod oracle_test;
