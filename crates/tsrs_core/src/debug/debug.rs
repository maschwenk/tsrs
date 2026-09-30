use std::fmt::Display;

pub fn fail(reason: &str) -> ! {
    let reason = if reason.is_empty() { "Debug failure.".to_string() } else { format!("Debug failure. {reason}") };
    panic!("{}", reason);
}

/// Go takes the node (for its `KindString()`) and optional message parts.
pub fn fail_bad_syntax_kind(kind_string: &str, message: &[&dyn Display]) -> ! {
    let msg = if message.is_empty() { "Unexpected node.".to_string() } else { message.iter().map(|m| m.to_string()).collect::<String>() };
    fail(&format!("{msg}\nNode {kind_string} was unexpected."));
}

pub fn assert_never(member: &dyn Display, message: &[&dyn Display]) -> ! {
    let msg = if message.is_empty() { "Illegal value:".to_string() } else { message.iter().map(|m| m.to_string()).collect::<String>() };
    fail(&format!("{msg} {member}"));
}

#[inline]
pub fn assert(value: bool, message: &[&dyn Display]) {
    if value {
        return;
    }
    assert_slow(message);
}

#[cold]
fn assert_slow(message: &[&dyn Display]) {
    let msg = if !message.is_empty() {
        format!("False expression: {}", message.iter().map(|m| m.to_string()).collect::<String>())
    } else {
        "False expression.".to_string()
    };
    fail(&msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panic_message(f: impl FnOnce() + std::panic::UnwindSafe) -> String {
        let err = std::panic::catch_unwind(f).unwrap_err();
        err.downcast_ref::<String>().cloned().unwrap_or_default()
    }

    #[test]
    fn messages() {
        assert_eq!(panic_message(|| fail("")), "Debug failure.");
        assert_eq!(panic_message(|| fail("x")), "Debug failure. x");
        assert_eq!(panic_message(|| assert(false, &[])), "Debug failure. False expression.");
        assert_eq!(panic_message(|| assert(false, &[&"a", &1])), "Debug failure. False expression: a1");
        assert_eq!(panic_message(|| assert_never(&"v", &[])), "Debug failure. Illegal value: v");
        assert_eq!(panic_message(|| fail_bad_syntax_kind("KindFoo", &[])), "Debug failure. Unexpected node.\nNode KindFoo was unexpected.");
        assert(true, &[]);
    }
}
