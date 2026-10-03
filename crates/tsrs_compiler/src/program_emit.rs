// `Program.Emit` and its types (compiler/program.go:1845-2016). Kept out of program.rs so the emit code and the
// LSP work on program.rs merge cheaply (docs/EMIT.md section 11).
//
// Threading (docs/EMIT.md section 10): Go queues one task per file on a work group; each task takes the file's
// checker (`newEmitHost`) for the whole emit of that file. Here, with the compiler's own checker pool, emit runs per
// checker group on the checker threads (files in program order within a group), the way `get_declaration_diagnostics`
// does; each file's result goes into a slot indexed by its position in the emit list, and `combine_emit_results`
// runs in input order, which is Go's observable order. With an external pool each file takes its checker from the
// pool, as in Go.

use std::sync::Mutex;

use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_core::P;

use crate::checkerpool::Context;
use crate::emitter::{emitter, EmitOnly};
use crate::program::{get_diagnostics_of_any_program, Program};
use tsrs_tsoptions::outputpaths::{self, ForceEmitPaths};

// program.go:1845
#[derive(Default)]
pub struct WriteFileData {
    pub source_map_url_pos: i32,
    // BuildInfo any: the incremental build info (TODO(emit/incremental)).
    pub diagnostics: Vec<P<Diagnostic>>,
    pub skipped_dts_write: bool,
    pub source_file: Option<P<SourceFile>>,
}

// program.go:1853. Called from the checker threads, hence `Sync`.
pub type WriteFile<'a> = &'a (dyn Fn(&str, &str, &mut WriteFileData) -> Result<(), String> + Sync);

// program.go:1855
#[derive(Default)]
pub struct EmitOptions<'a> {
    pub target_source_files: Option<Vec<P<SourceFile>>>, // Source files to emit. If `None`, emits all files
    pub emit_only: EmitOnly,
    pub force_emit: bool,
    pub write_file: Option<WriteFile<'a>>,
}

// program.go:1862
#[derive(Default, Clone)]
pub struct EmitResult {
    pub emit_skipped: bool,
    pub diagnostics: Vec<P<Diagnostic>>,            // Contains declaration emit diagnostics
    pub emitted_files: Vec<String>,                 // Array of files the compiler wrote to disk
    pub source_maps: Vec<SourceMapEmitResult>,      // Array of sourceMapData if compiler emitted sourcemaps
}

// program.go:1869
#[derive(Default, Clone)]
pub struct SourceMapEmitResult {
    pub input_source_file_names: Vec<String>, // Input source file (which one can use on program to get the file), 1:1 mapping with the sourceMap.sources list
    // SourceMap *sourcemap.RawSourceMap (TODO(emit/sourcemaps))
    pub generated_file: String,
}

impl Program {
    // program.go:1875
    pub fn emit(&'static self, ctx: &Context, options: EmitOptions) -> EmitResult {
        if !options.force_emit && options.emit_only != EmitOnly::EmitOnlyBuilderSignature {
            let result = handle_no_emit_options(ctx, self, options.target_source_files.as_deref(), None);
            if let Some(result) = result {
                return result;
            }
        }

        let new_line = self.options().new_line.get_new_line_character();
        let force_dts_emit = options.emit_only == EmitOnly::EmitOnlyBuilderSignature || options.force_emit && options.emit_only == EmitOnly::EmitOnlyDts;
        let force_js_emit = options.force_emit && options.emit_only == EmitOnly::EmitOnlyJs;
        let source_files = crate::emitter::get_source_files_to_emit(self, options.target_source_files.as_deref(), force_dts_emit, force_js_emit);

        let results: Vec<Mutex<Option<EmitResult>>> = source_files.iter().map(|_| Mutex::new(None)).collect();
        let run = |c: &mut tsrs_checker::Checker, index: usize, source_file: P<SourceFile>| {
            let checker_slot = P::new(tsrs_checker::CheckerSlot::default());
            let host = crate::emithost::new_emit_host(self, c.get_emit_resolver(), checker_slot);

            // take an unused writer (Go pools them; a fresh writer prints the same text)
            let writer = tsrs_printer::new_text_writer(new_line, 0);
            let paths = outputpaths::get_output_paths_for(
                source_file,
                &self.options(),
                host,
                ForceEmitPaths { dts: force_dts_emit, js: force_js_emit, declaration_map: options.force_emit && options.emit_only == EmitOnly::EmitOnlyDts },
            );
            let mut e = emitter {
                host,
                emit_only: options.emit_only,
                emitter_diagnostics: Default::default(),
                writer,
                paths,
                source_file,
                emit_result: EmitResult::default(),
                force_emit: options.force_emit,
                write_file: options.write_file,
            };
            checker_slot.lend(c, || e.emit());
            *results[index].lock().unwrap() = Some(e.emit_result);
        };

        match self.compiler_checker_pool() {
            Some(pool) => pool.for_each_checker_group_do(&source_files, self.single_threaded(), |c, index, file| run(c, index, file)),
            None => {
                for (index, &file) in source_files.iter().enumerate() {
                    let mut guard = self.get_type_checker_for_file(ctx, file);
                    run(&mut guard, index, file);
                }
            }
        }

        // collect results from emit, preserving input order
        combine_emit_results(results.into_iter().map(|r| r.into_inner().unwrap()).collect())
    }
}

// program.go:1947
pub fn combine_emit_results(results: Vec<Option<EmitResult>>) -> EmitResult {
    let mut result = EmitResult::default();
    for emit_result in results {
        let Some(emit_result) = emit_result else {
            continue; // Skip nil results
        };
        if emit_result.emit_skipped {
            result.emit_skipped = true;
        }
        result.diagnostics.extend(emit_result.diagnostics);
        result.emitted_files.extend(emit_result.emitted_files);
        result.source_maps.extend(emit_result.source_maps);
    }
    result
}

// program.go:1984. `program` is Go's `ProgramLike`; the incremental program (TODO(emit/incremental)) passes its own
// `emitBuildInfo`.
pub fn handle_no_emit_options(ctx: &Context, program: &'static Program, files: Option<&[P<SourceFile>]>, emit_build_info: Option<&dyn Fn() -> Option<EmitResult>>) -> Option<EmitResult> {
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
        let result = emit_build_info();
        if result.is_some() {
            return result;
        }
    }
    Some(EmitResult::default())
}
