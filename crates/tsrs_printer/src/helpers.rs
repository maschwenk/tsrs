// Only the EmitHelper data type and its ordering are ported: helper definitions and the transforms that request
// them are emit-only. The printer still sorts and writes whatever helpers an EmitContext carries.

pub struct Priority {
    pub value: i32,
}

pub struct EmitHelper {
    pub name: &'static str, // A unique name for this helper.
    pub scoped: bool, // Indicates whether the helper MUST be emitted in the current scope.
    pub text: &'static str, // ES3-compatible raw script text
    pub text_callback: Option<fn(make_unique_name: &mut dyn FnMut(&str) -> String) -> String>, // A function yielding an ES3-compatible raw script text.
    pub priority: Option<&'static Priority>, // Helpers with a higher priority are emitted earlier than other helpers on the node.
    pub dependencies: &'static [tsrs_core::P<EmitHelper>], // Emit helpers this helper depends on
    pub import_name: &'static str, // The name of the helper to use when importing via `--importHelpers`.
}

pub(crate) fn compare_emit_helpers(x: tsrs_core::P<EmitHelper>, y: tsrs_core::P<EmitHelper>) -> i32 {
    if x == y {
        return 0;
    }
    let same_priority = match (x.priority, y.priority) {
        (None, None) => true,
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        _ => false,
    };
    if same_priority {
        return 0;
    }
    if x.priority.is_none() {
        return 1;
    }
    if y.priority.is_none() {
        return -1;
    }
    x.priority.unwrap().value - y.priority.unwrap().value
}
