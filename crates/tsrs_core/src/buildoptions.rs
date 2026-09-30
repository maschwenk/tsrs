use crate::Tristate;

#[derive(Clone, Debug, Default)]
pub struct BuildOptions {
    pub dry: Tristate,
    pub force: Tristate,
    pub verbose: Tristate,
    pub builders: Option<i32>,
    pub stop_build_on_errors: Tristate,

    // CompilerOptions are not parsed here and will be available on ParsedBuildCommandLine

    // Internal fields
    pub clean: Tristate,
}
