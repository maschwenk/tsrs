// The emit surface of Go's compiler package that the incremental program uses (compiler/program.go:1845-2073,
// compiler/emitter.go:24).
//
// TODO(emit/core): `Program.Emit`, `EmitOptions`, `EmitResult`, `WriteFileData`, `ProgramLike`,
// `CombineEmitResults` and `HandleNoEmitOptions` belong in tsrs_compiler (wave E1). Until that branch has landed,
// they live here and `Program.Emit` writes nothing: the builder-signature emit produces no `.d.ts` text, so file
// signatures fall back to the file version (Go's `useFileVersionAsSignature` path).

use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_compiler::{Context, Program as CompilerProgram};
use tsrs_core::{CompilerOptions, P};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EmitOnly {
    #[default]
    EmitAll,
    EmitOnlyJs,
    EmitOnlyDts,
    EmitOnlyBuilderSignature,
}

pub struct WriteFileData {
    pub source_map_url_pos: i32,
    pub build_info: Option<crate::buildinfo::BuildInfo>,
    pub diagnostics: Vec<P<Diagnostic>>,
    pub skipped_dts_write: bool,
    pub source_file: Option<P<SourceFile>>,
}

impl Default for WriteFileData {
    fn default() -> Self {
        WriteFileData { source_map_url_pos: -1, build_info: None, diagnostics: Vec::new(), skipped_dts_write: false, source_file: None }
    }
}

pub type WriteFile<'a> = &'a dyn Fn(&str, &str, &mut WriteFileData) -> Result<(), String>;

#[derive(Clone, Default)]
pub struct EmitOptions<'a> {
    pub target_source_files: Option<Vec<P<SourceFile>>>, // Source files to emit. If `nil`, emits all files
    pub emit_only: EmitOnly,
    pub force_emit: bool,
    pub write_file: Option<WriteFile<'a>>,
}

#[derive(Clone, Debug, Default)]
pub struct EmitResult {
    pub emit_skipped: bool,
    pub diagnostics: Vec<P<Diagnostic>>, // Contains declaration emit diagnostics
    pub emitted_files: Vec<String>,      // Array of files the compiler wrote to disk
}

// program.go:1947
pub fn combine_emit_results(results: Vec<EmitResult>) -> EmitResult {
    let mut result = EmitResult::default();
    for emit_result in results {
        if emit_result.emit_skipped {
            result.emit_skipped = true;
        }
        result.diagnostics.extend(emit_result.diagnostics);
        result.emitted_files.extend(emit_result.emitted_files);
    }
    result
}

// compiler.ProgramLike (program.go:1965).
pub trait ProgramLike {
    fn options(&self) -> P<CompilerOptions>;
    fn get_source_file(&self, path: &str) -> Option<P<SourceFile>>;
    fn get_source_files(&self) -> &'static [P<SourceFile>];
    fn get_config_file_parsing_diagnostics(&self) -> Vec<P<Diagnostic>>;
    fn get_syntactic_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>>;
    fn get_bind_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>>;
    fn get_program_diagnostics(&self) -> Vec<P<Diagnostic>>;
    fn get_global_diagnostics(&self, ctx: &Context) -> Vec<P<Diagnostic>>;
    fn get_semantic_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>>;
    fn get_declaration_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>>;
    fn get_suggestion_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>>;
    fn emit(&self, ctx: &Context, options: EmitOptions) -> Option<EmitResult>;
    fn common_source_directory(&self) -> String;
    fn is_source_file_default_library(&self, path: &tsrs_core::tspath::Path) -> bool;
    fn program(&self) -> &'static CompilerProgram;
    // Go's `program.(*compiler.Program)` type assertion in GetDiagnosticsOfAnyProgram.
    fn is_compiler_program(&self) -> bool;
}

impl ProgramLike for &'static CompilerProgram {
    fn options(&self) -> P<CompilerOptions> {
        CompilerProgram::options(self)
    }
    fn get_source_file(&self, path: &str) -> Option<P<SourceFile>> {
        CompilerProgram::get_source_file(self, path)
    }
    fn get_source_files(&self) -> &'static [P<SourceFile>] {
        CompilerProgram::get_source_files(self)
    }
    fn get_config_file_parsing_diagnostics(&self) -> Vec<P<Diagnostic>> {
        CompilerProgram::get_config_file_parsing_diagnostics(self)
    }
    fn get_syntactic_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        CompilerProgram::get_syntactic_diagnostics(self, ctx, file)
    }
    fn get_bind_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        CompilerProgram::get_bind_diagnostics(self, ctx, file)
    }
    fn get_program_diagnostics(&self) -> Vec<P<Diagnostic>> {
        CompilerProgram::get_program_diagnostics(self)
    }
    fn get_global_diagnostics(&self, ctx: &Context) -> Vec<P<Diagnostic>> {
        CompilerProgram::get_global_diagnostics(self, ctx)
    }
    fn get_semantic_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        CompilerProgram::get_semantic_diagnostics(self, ctx, file)
    }
    fn get_declaration_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        CompilerProgram::get_declaration_diagnostics(self, ctx, file)
    }
    fn get_suggestion_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        CompilerProgram::get_suggestion_diagnostics(self, ctx, file)
    }
    fn emit(&self, ctx: &Context, options: EmitOptions) -> Option<EmitResult> {
        compiler_program_emit(self, ctx, options)
    }
    fn common_source_directory(&self) -> String {
        CompilerProgram::common_source_directory(self).to_string()
    }
    fn is_source_file_default_library(&self, path: &tsrs_core::tspath::Path) -> bool {
        CompilerProgram::is_source_file_default_library(self, path)
    }
    fn program(&self) -> &'static CompilerProgram {
        self
    }
    fn is_compiler_program(&self) -> bool {
        true
    }
}

// TODO(emit/core): Go `(*compiler.Program).Emit` (program.go:1875). Nothing is printed yet: with
// `EmitOnlyBuilderSignature` no `.d.ts` reaches `WriteFile`, so callers fall back to the file version.
pub fn compiler_program_emit(program: &'static CompilerProgram, ctx: &Context, options: EmitOptions) -> Option<EmitResult> {
    if !options.force_emit && options.emit_only != EmitOnly::EmitOnlyBuilderSignature {
        if let Some(result) = handle_no_emit_options(ctx, &program, options.target_source_files.as_deref(), None::<&dyn Fn() -> Option<EmitResult>>) {
            return Some(result);
        }
    }
    Some(EmitResult::default())
}

// program.go:1984
// HandleNoEmitOptions mirrors tsc's handleNoEmitOptions.
pub fn handle_no_emit_options(
    ctx: &Context,
    program: &dyn ProgramLike,
    files: Option<&[P<SourceFile>]>,
    emit_build_info: Option<&dyn Fn() -> Option<EmitResult>>,
) -> Option<EmitResult> {
    if !program.options().no_emit.is_true() {
        if !program.options().no_emit_on_error.is_true() {
            return None; // NoEmit is false and NoEmitOnError is also false, so we can proceed with normal emit
        }

        let diagnostics = get_diagnostics_of_any_program(
            ctx,
            program,
            files,
            true,
            &mut |ctx, file| program.get_bind_diagnostics(ctx, file),
            &mut |ctx, file| program.get_semantic_diagnostics(ctx, file),
        );
        if diagnostics.is_empty() {
            return None; // NoEmitOnError is enabled, but no diagnostics were found, so we can proceed with emitting
        }
        return Some(EmitResult { diagnostics, emit_skipped: true, ..Default::default() });
    }
    if files.is_some() {
        return Some(EmitResult { emit_skipped: true, ..Default::default() });
    }
    if let Some(emit_build_info) = emit_build_info {
        if let Some(result) = emit_build_info() {
            return Some(result);
        }
    }
    Some(EmitResult::default())
}

// program.go:2018
pub fn get_diagnostics_of_any_program(
    ctx: &Context,
    program: &dyn ProgramLike,
    files: Option<&[P<SourceFile>]>,
    skip_no_emit_check_for_dts_diagnostics: bool,
    get_bind_diagnostics: &mut dyn FnMut(&Context, Option<P<SourceFile>>) -> Vec<P<Diagnostic>>,
    get_semantic_diagnostics: &mut dyn FnMut(&Context, Option<P<SourceFile>>) -> Vec<P<Diagnostic>>,
) -> Vec<P<Diagnostic>> {
    let mut all_diagnostics = program.get_config_file_parsing_diagnostics();
    let config_file_parsing_diagnostics_length = all_diagnostics.len();

    let append_diagnostics_for_all_files =
        |diagnostics: &mut Vec<P<Diagnostic>>, get_diagnostics: &mut dyn FnMut(&Context, Option<P<SourceFile>>) -> Vec<P<Diagnostic>>| match files {
            None => diagnostics.extend(get_diagnostics(ctx, None)),
            Some(files) => {
                for &file in files {
                    diagnostics.extend(get_diagnostics(ctx, Some(file)));
                }
            }
        };

    let mut syntactic_diagnostics = Vec::new();
    append_diagnostics_for_all_files(&mut syntactic_diagnostics, &mut |ctx, f| program.get_syntactic_diagnostics(ctx, f));
    all_diagnostics.extend(syntactic_diagnostics);

    // If we didn't have any syntactic errors, then also try getting the program (options),
    // global and semantic errors.
    if all_diagnostics.len() == config_file_parsing_diagnostics_length {
        all_diagnostics.extend(program.get_program_diagnostics());

        // Do binding early so we can track the time.
        append_diagnostics_for_all_files(&mut Vec::new(), get_bind_diagnostics);

        if program.options().list_files_only.is_false_or_unknown() {
            all_diagnostics.extend(program.get_global_diagnostics(ctx));

            if all_diagnostics.len() == config_file_parsing_diagnostics_length {
                append_diagnostics_for_all_files(&mut all_diagnostics, get_semantic_diagnostics);
                if program.is_compiler_program() {
                    // Incremental programs cache checking globals with file diagnostics;
                    // a late sweep would also collect incidental signature-generation globals.
                    all_diagnostics.extend(program.get_global_diagnostics(ctx));
                }
            }

            if (skip_no_emit_check_for_dts_diagnostics || program.options().no_emit.is_true())
                && program.options().get_emit_declarations()
                && all_diagnostics.len() == config_file_parsing_diagnostics_length
            {
                append_diagnostics_for_all_files(&mut all_diagnostics, &mut |ctx, f| program.get_declaration_diagnostics(ctx, f));
            }
        }
    }
    all_diagnostics
}
