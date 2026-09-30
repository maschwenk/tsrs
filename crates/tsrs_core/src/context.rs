// Go threads these through context.Context; the Rust port passes them explicitly where needed.

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum CheckerLifetime {
    #[default]
    Temporary = 0,
    Diagnostics = 1,
    API = 2,
}
