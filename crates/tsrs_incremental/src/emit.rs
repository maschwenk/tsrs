// The emit surface of Go's compiler package that the incremental program uses: `ProgramLike`, `HandleNoEmitOptions`
// and `GetDiagnosticsOfAnyProgram` over it live in tsrs_compiler (programlike.rs, program_emit.rs; emit/core-2).

use tsrs_compiler::{Context, Program as CompilerProgram};

pub use tsrs_compiler::{
    combine_emit_results, get_diagnostics_of_any_program_like as get_diagnostics_of_any_program, handle_no_emit_options, EmitOnly, EmitOptions,
    EmitResult, ProgramLike, WriteFile, WriteFileData,
};

// Go `(*compiler.Program).Emit` (program.go:1875).
pub fn compiler_program_emit(program: &CompilerProgram, ctx: &Context, options: &EmitOptions) -> Option<EmitResult> {
    Some(program.emit(ctx, options))
}
