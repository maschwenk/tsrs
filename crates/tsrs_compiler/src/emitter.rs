// Only the parts of Go's emitter.go that type checking depends on. Emit itself is not ported.

use tsrs_ast::{self as ast, SourceFile};
use tsrs_core::tspath::{self, ComparePathsOptions};
use tsrs_core::P;

use crate::outputpaths;
use crate::program::Program;

pub(crate) fn source_file_may_be_emitted(source_file: P<SourceFile>, host: &Program, force_dts_emit: bool, force_js_emit: bool) -> bool {
    // TODO: move this to outputpaths?
    let options = host.options();
    // Js files are emitted only if option is enabled
    if !force_js_emit && options.no_emit_for_js_files.is_true() && ast::is_source_file_js(source_file) {
        return false;
    }

    // Declaration files are not emitted
    if source_file.is_declaration_file.get() {
        return false;
    }

    // Source file from node_modules are not emitted
    if host.is_source_file_from_external_library(source_file) {
        return false;
    }

    // forcing dts emit => file needs to be emitted
    if force_dts_emit || force_js_emit {
        return true;
    }

    // Check other conditions for file emit
    // Source files from referenced projects are not emitted
    if host.get_project_reference_from_source(&source_file.path()).is_some() {
        return false;
    }

    // Any non json file should be emitted
    if !ast::is_json_source_file(source_file) {
        return true;
    }

    // Json file is not emitted if outDir is not specified
    if options.out_dir.is_empty() {
        return false;
    }

    // Otherwise, if rootDir is specified or a config file exists, we know the common source directory and can check if the file would be emitted in the same location
    if !options.root_dir.is_empty() || !options.config_file_path.is_empty() {
        let common_dir = tspath::get_normalized_absolute_path(
            &outputpaths::get_common_source_directory(
                &options,
                Vec::new,
                host.get_current_directory(),
                host.use_case_sensitive_file_names(),
                None,
            ),
            host.get_current_directory(),
        );
        let output_path = outputpaths::get_source_file_path_in_new_dir_worker(
            source_file.file_name(),
            &options.out_dir,
            host.get_current_directory(),
            &common_dir,
            host.use_case_sensitive_file_names(),
        );
        if tspath::compare_paths(
            source_file.file_name(),
            &output_path,
            &ComparePathsOptions {
                use_case_sensitive_file_names: host.use_case_sensitive_file_names(),
                current_directory: host.get_current_directory().to_string(),
            },
        ) == 0
        {
            return false;
        }
    }

    true
}

pub(crate) fn get_source_files_to_emit(
    host: &Program,
    target_source_files: Option<&[P<SourceFile>]>,
    force_dts_emit: bool,
    force_js_emit: bool,
) -> Vec<P<SourceFile>> {
    let target_source_files = target_source_files.unwrap_or_else(|| host.source_files());
    target_source_files.iter().copied().filter(|&f| source_file_may_be_emitted(f, host, force_dts_emit, force_js_emit)).collect()
}

fn is_source_file_not_json(file: P<SourceFile>) -> bool {
    !ast::is_json_source_file(file)
}

// emitter.go:566. Runs inside the caller's `CheckerSlot::lend` (the host's resolver borrows the checker from it).
#[cfg(feature = "checker")]
pub(crate) fn get_declaration_diagnostics(host: &'static crate::emithost::EmitHost, program: &Program, file: P<SourceFile>) -> Vec<P<tsrs_ast::Diagnostic>> {
    // TODO: use p.getSourceFilesToEmit cache
    // Go passes the emit host as the SourceFileMayBeEmittedHost; its methods forward to the program.
    let full_files: Vec<P<SourceFile>> = get_source_files_to_emit(program, Some(&[file]), false, false).into_iter().filter(|&f| is_source_file_not_json(f)).collect();
    if !full_files.iter().any(|&f| f == file) {
        return Vec::new();
    }
    let options = program.options();
    let transform = tsrs_declarations::new_declaration_transformer(host, None, options, "", "");
    transform.base.transform_source_file(file);
    transform.get_diagnostics()
}

// ---------------------------------------------------------------------------------------------------------------
// The emitter (emitter.go:23 onwards, minus the parts above). Only built with the checker: emit needs the emit
// resolver. Nothing here runs unless a caller invokes `Program::emit` (the CLI does unless the options disable emit).
// ---------------------------------------------------------------------------------------------------------------

#[cfg(feature = "checker")]
pub use self::emit::*;

#[cfg(feature = "checker")]
mod emit {
    use tsrs_ast::{self as ast, new_compiler_diagnostic, Diagnostic, DiagnosticsCollection, SourceFile};
    use tsrs_core::tspath::{self, ComparePathsOptions};
    use tsrs_core::{stringutil, CompilerOptions, LanguageVariant, ModuleKind, NewLineKind, ScriptTarget, Tristate, P};
    use tsrs_diagnostics as diagnostics;
    use tsrs_printer::{self as printer, EmitContext, EmitTextWriter, PrintHandlers, Printer, PrinterOptions, SourceMapGenerator};
    use tsrs_transformers::{
        self as transformers, estransforms, inliners, jsxtransforms, moduletransforms, tstransforms, EmitHost as _, ReferenceResolverRef, TransformOptions, Transformer,
    };
    use tsrs_tsoptions::outputpaths::{self, OutputPaths};

    use crate::emithost::EmitHost;
    use crate::program_emit::{EmitPhase, EmitResult, EmitTimes, SourceMapEmitResult, WriteFile, WriteFileData};

    // emitter.go:24
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub enum EmitOnly {
        #[default]
        EmitAll,
        EmitOnlyJs,
        EmitOnlyDts,
        EmitOnlyBuilderSignature,
    }

    // emitter.go:33
    pub(crate) struct emitter<'a> {
        pub(crate) host: &'static EmitHost,
        pub(crate) emit_only: EmitOnly,
        pub(crate) emitter_diagnostics: DiagnosticsCollection,
        pub(crate) writer: Box<dyn EmitTextWriter>,
        pub(crate) paths: OutputPaths,
        pub(crate) source_file: P<SourceFile>,
        pub(crate) emit_result: EmitResult,
        pub(crate) force_emit: bool,
        pub(crate) write_file: Option<WriteFile<'a>>,
        pub(crate) times: &'a EmitTimes,
    }

    impl emitter<'_> {
        // emitter.go:46
        pub(crate) fn emit(&mut self) {
            let js_file_path = self.paths.js_file_path().to_string();
            let source_map_file_path = self.paths.source_map_file_path().to_string();
            self.emit_js_file(Some(self.source_file), &js_file_path, &source_map_file_path);
            let declaration_file_path = self.paths.declaration_file_path().to_string();
            let declaration_map_path = self.paths.declaration_map_path().to_string();
            self.emit_declaration_file(Some(self.source_file), &declaration_file_path, &declaration_map_path);
            self.emit_result.diagnostics = self.emitter_diagnostics.get_diagnostics();
        }

        // emitter.go:62. Go returns a slice of the `declarationTransformer` interface; the two transformers run in
        // `run_declaration_transformers` in the same order.
        fn run_declaration_transformers(&mut self, emit_context: P<EmitContext>, source_file: P<SourceFile>, declaration_file_path: &str, declaration_map_path: &str) -> (P<SourceFile>, Vec<P<Diagnostic>>) {
            let force_dts_emit = self.emit_only == EmitOnly::EmitOnlyBuilderSignature || self.force_emit && self.emit_only == EmitOnly::EmitOnlyDts;
            let mut diags = Vec::new();
            // getDeclarationTransformers (emitter.go:62)
            let declaration = tsrs_declarations::new_declaration_transformer(self.host, Some(emit_context), self.host.options(), declaration_file_path, declaration_map_path);
            let supplemental = tsrs_declarations::new_supplemental_references_transformer(self.host, source_file, declaration_file_path, force_dts_emit);
            // emitter.go:81
            let source_file = self.times.time(EmitPhase::DeclarationTransform, || {
                let source_file = declaration.base.transform_source_file(source_file);
                diags.extend(declaration.get_diagnostics());
                let source_file = supplemental.transform_source_file(source_file);
                diags.extend(supplemental.get_diagnostics());
                source_file
            });
            (source_file, diags)
        }

        // emitter.go:71
        fn run_script_transformers(&mut self, emit_context: P<EmitContext>, mut source_file: P<SourceFile>) -> P<SourceFile> {
            let host = self.host;
            self.times.time(EmitPhase::ScriptTransform, || {
                for transformer in get_script_transformers(emit_context, host, source_file) {
                    source_file = transformer.transform_source_file(source_file);
                }
                source_file
            })
        }

        // emitter.go:192
        fn emit_js_file(&mut self, source_file: Option<P<SourceFile>>, js_file_path: &str, source_map_file_path: &str) {
            let options = self.host.options();

            let Some(source_file) = source_file else {
                return;
            };
            if self.emit_only != EmitOnly::EmitAll && self.emit_only != EmitOnly::EmitOnlyJs || js_file_path.is_empty() {
                return;
            }

            if !self.force_emit && (options.no_emit == Tristate::True || self.host.is_emit_blocked(js_file_path)) {
                self.emit_result.emit_skipped = true;
                return;
            }

            let (emit_context, put_emit_context) = printer::get_emit_context();

            let source_file = self.run_script_transformers(emit_context, source_file);

            let printer_options = PrinterOptions {
                remove_comments: options.remove_comments.is_true(),
                new_line: options.new_line,
                no_emit_helpers: options.no_emit_helpers.is_true(),
                source_map: options.source_map.is_true(),
                inline_source_map: options.inline_source_map.is_true(),
                inline_sources: options.inline_sources.is_true(),
                target: options.target,
                // !!!
                ..Default::default()
            };

            // create a printer to print the nodes
            let printer = printer::new_printer(
                printer_options,
                PrintHandlers {
                    // !!!
                    ..Default::default()
                },
                Some(emit_context),
            );

            self.print_source_file(js_file_path, source_map_file_path, source_file, printer, &options, should_emit_source_maps(&options, source_file));
            put_emit_context();
        }

        // emitter.go:239
        fn emit_declaration_file(&mut self, source_file: Option<P<SourceFile>>, declaration_file_path: &str, declaration_map_path: &str) {
            let options = self.host.options();

            let Some(source_file) = source_file else {
                return;
            };
            if self.emit_only == EmitOnly::EmitOnlyJs || declaration_file_path.is_empty() {
                return;
            }
            let emit_declaration_map = self.emit_only != EmitOnly::EmitOnlyBuilderSignature && options.declaration_map.is_true();
            let content_mapped_source = source_file;

            let (emit_context, put_emit_context) = printer::get_emit_context();
            let (source_file, diags) = self.run_declaration_transformers(emit_context, source_file, declaration_file_path, declaration_map_path);

            for &elem in &diags {
                // Add declaration transform diagnostics to emit diagnostics
                self.emitter_diagnostics.add(elem);
            }

            if !self.force_emit && self.emit_only != EmitOnly::EmitOnlyBuilderSignature && (options.no_emit == Tristate::True || self.host.is_emit_blocked(declaration_file_path)) {
                self.emit_result.emit_skipped = true;
                put_emit_context();
                return;
            }

            let decl_blocked = !diags.is_empty() && !self.force_emit && self.emit_only != EmitOnly::EmitOnlyBuilderSignature;
            if decl_blocked {
                self.emit_result.emit_skipped = true;
                put_emit_context();
                return;
            }

            let printer_options = PrinterOptions {
                remove_comments: options.remove_comments.is_true(),
                new_line: options.new_line,
                no_emit_helpers: true,
                // Module: 			   options.Module, // NYI
                // ModuleResolution:   options.ModuleResolution, // NYI
                target: options.get_emit_script_target(),
                source_map: emit_declaration_map,
                inline_source_map: options.inline_source_map.is_true(),
                // InlineSources:       options.InlineSources.IsTrue(), // ignored, per strada
                // ExtendedDiagnostics: options.ExtendedDiagnostics.IsTrue(), // NYI
                only_print_js_doc_style: true,
                omit_brace_source_map_positions: true,
                ..Default::default()
            };

            // create a printer to print the nodes
            // Go installs PrintHandlers.MapSourcePosition when the file has a content-mapper span map; content mappers
            // are not supported by tsrs (docs/EMIT.md section 10), so the handlers stay empty.
            let _ = content_mapped_source;
            let print_handlers = PrintHandlers::default();
            let printer = printer::new_printer(printer_options, print_handlers, Some(emit_context));

            let declaration_map_options = CompilerOptions {
                source_map: if emit_declaration_map { Tristate::True } else { Tristate::False },
                source_root: options.source_root.clone(),
                map_root: options.map_root.clone(),
                // Explicitly do not pass through either inline option.
                ..Default::default()
            };
            let should_emit = should_emit_source_maps(&declaration_map_options, source_file);
            self.print_source_file(declaration_file_path, declaration_map_path, source_file, printer, &declaration_map_options, should_emit);
            put_emit_context();
        }

        // emitter.go:345
        fn print_source_file(&mut self, js_file_path: &str, source_map_file_path: &str, source_file: P<SourceFile>, mut printer_: Printer, map_options: &CompilerOptions, should_emit_source_maps: bool) {
            // !!! sourceMapGenerator
            let options = self.host.options();
            let mut source_map_generator: Option<SourceMapGenerator> = None;
            if should_emit_source_maps {
                source_map_generator = Some(tsrs_sourcemap::new_generator(
                    &tspath::get_base_file_name(&tspath::normalize_slashes(js_file_path)),
                    &get_source_root(map_options),
                    &self.get_source_map_directory(map_options, js_file_path, Some(source_file)),
                    ComparePathsOptions { use_case_sensitive_file_names: self.host.use_case_sensitive_file_names(), current_directory: self.host.get_current_directory().to_string() },
                ));
            }

            let print_start = std::time::Instant::now();
            printer_.write(source_file.as_node(), Some(source_file), &mut *self.writer, source_map_generator.as_mut());
            self.times.add(EmitPhase::Print, print_start);

            let mut source_map_url_pos: i32 = -1;
            if let Some(source_map_generator) = &mut source_map_generator {
                let source_map_start = std::time::Instant::now();
                if map_options.source_map.is_true() || map_options.inline_source_map.is_true() {
                    self.emit_result.source_maps.push(SourceMapEmitResult {
                        input_source_file_names: source_map_generator.sources().to_vec(),
                        source_map: source_map_generator.raw_source_map(),
                        generated_file: js_file_path.to_string(),
                    });
                }

                let source_mapping_url = self.get_source_mapping_url(map_options, source_map_generator, js_file_path, source_map_file_path, Some(source_file));

                if !source_mapping_url.is_empty() {
                    if !self.writer.is_at_start_of_line() {
                        self.writer.raw_write(if options.new_line == NewLineKind::CRLF { "\r\n" } else { "\n" });
                    }
                    source_map_url_pos = self.writer.get_text_pos();
                    self.writer.write_comment("//# sourceMappingURL=");
                    self.writer.write_comment(&source_mapping_url);
                }

                // Write the source map
                if !source_map_file_path.is_empty() {
                    let source_map = source_map_generator.string();
                    self.times.add(EmitPhase::SourceMap, source_map_start);
                    let err = self.write_text(source_map_file_path, &source_map, &mut WriteFileData { source_file: Some(self.source_file), ..Default::default() });
                    match err {
                        Err(err) => {
                            self.emitter_diagnostics.add(new_compiler_diagnostic(&diagnostics::Could_not_write_file_0_Colon_1, &[&js_file_path, &err]));
                        }
                        Ok(()) => {
                            self.emit_result.emitted_files.push(source_map_file_path.to_string());
                        }
                    }
                } else {
                    self.times.add(EmitPhase::SourceMap, source_map_start);
                }
            } else {
                self.writer.write_line();
            }

            // Write the output file
            let mut text = self.writer.string();
            if options.emit_bom.is_true() {
                text = stringutil::add_utf8_byte_order_mark(&text);
            }
            let mut data = WriteFileData {
                source_map_url_pos,
                diagnostics: self.emitter_diagnostics.get_diagnostics(),
                source_file: Some(self.source_file),
                ..Default::default()
            };
            let err = self.write_text(js_file_path, &text, &mut data);
            let skipped_dts_write = data.skipped_dts_write;
            match err {
                Err(err) => {
                    self.emitter_diagnostics.add(new_compiler_diagnostic(&diagnostics::Could_not_write_file_0_Colon_1, &[&js_file_path, &err]));
                }
                Ok(()) if !skipped_dts_write => {
                    self.emit_result.emitted_files.push(js_file_path.to_string());
                }
                Ok(()) => {}
            }

            // Reset state
            self.writer.clear();
        }

        // emitter.go:436
        fn write_text(&self, file_name: &str, text: &str, data: &mut WriteFileData) -> Result<(), String> {
            self.times.time(EmitPhase::Write, || {
                if let Some(write_file) = self.write_file {
                    return write_file(file_name, text, data);
                }
                self.host.write_file(file_name, text)
            })
        }

        // emitter.go:460
        fn get_source_map_directory(&self, map_options: &CompilerOptions, file_path: &str, source_file: Option<P<SourceFile>>) -> String {
            if !map_options.source_root.is_empty() {
                return self.host.common_source_directory();
            }
            if !map_options.map_root.is_empty() {
                let mut source_map_dir = tspath::normalize_slashes(&map_options.map_root);
                if let Some(source_file) = source_file {
                    // For modules or multiple emit files the mapRoot will have directory structure like the sources
                    // So if src\a.ts and src\lib\b.ts are compiled together user would be moving the maps into mapRoot\a.js.map and mapRoot\lib\b.js.map
                    source_map_dir = tspath::get_directory_path(&outputpaths::get_source_file_path_in_new_dir(
                        source_file.file_name(),
                        &source_map_dir,
                        self.host.get_current_directory(),
                        &self.host.common_source_directory(),
                        self.host.use_case_sensitive_file_names(),
                    ))
                    .to_string();
                }
                if tspath::get_root_length(&source_map_dir) == 0 {
                    // The relative paths are relative to the common directory
                    source_map_dir = tspath::combine_paths(&self.host.common_source_directory(), &[&source_map_dir]);
                }
                return source_map_dir;
            }
            tspath::get_directory_path(&tspath::normalize_path(file_path)).to_string()
        }

        // emitter.go:443
        pub(crate) fn get_source_mapping_url(
            &self,
            map_options: &CompilerOptions,
            source_map_generator: &mut SourceMapGenerator,
            file_path: &str,
            source_map_file_path: &str,
            source_file: Option<P<SourceFile>>,
        ) -> String {
            if map_options.inline_source_map.is_true() {
                // Encode the sourceMap into the sourceMap url
                return source_map_generator.base64_data_url();
            }

            let source_map_file = tspath::get_base_file_name(&tspath::normalize_slashes(source_map_file_path)).to_string();
            if !map_options.map_root.is_empty() {
                let mut source_map_dir = tspath::normalize_slashes(&map_options.map_root);
                if let Some(source_file) = source_file {
                    // For modules or multiple emit files the mapRoot will have directory structure like the sources
                    // So if src\a.ts and src\lib\b.ts are compiled together user would be moving the maps into mapRoot\a.js.map and mapRoot\lib\b.js.map
                    source_map_dir = tspath::get_directory_path(&outputpaths::get_source_file_path_in_new_dir(
                        source_file.file_name(),
                        &source_map_dir,
                        self.host.get_current_directory(),
                        &self.host.common_source_directory(),
                        self.host.use_case_sensitive_file_names(),
                    ))
                    .to_string();
                }
                if tspath::get_root_length(&source_map_dir) == 0 {
                    // The relative paths are relative to the common directory
                    source_map_dir = tspath::combine_paths(&self.host.common_source_directory(), &[&source_map_dir]);
                    return stringutil::encode_uri(&tspath::get_relative_path_to_directory_or_url(
                        &tspath::get_directory_path(&tspath::normalize_path(file_path)), // get the relative sourceMapDir path based on jsFilePath
                        &tspath::combine_paths(&source_map_dir, &[&source_map_file]), // this is where user expects to see sourceMap
                        /*isAbsolutePathAnUrl*/ true,
                        &ComparePathsOptions {
                            use_case_sensitive_file_names: self.host.use_case_sensitive_file_names(),
                            current_directory: self.host.get_current_directory().to_string(),
                        },
                    ));
                } else {
                    return stringutil::encode_uri(&tspath::combine_paths(&source_map_dir, &[&source_map_file]));
                }
            }
            stringutil::encode_uri(&source_map_file)
        }
    }

    // emitter.go:295
    // Go `declarationMapSource`: the original (content-mapped) source a declaration map points at.
    pub(crate) struct declarationMapSource {
        pub(crate) file_name: String,
        pub(crate) text: &'static str,
        pub(crate) line_map: Vec<tsrs_core::TextPos>,
    }

    // emitter.go:301
    pub(crate) fn new_declaration_map_source(source_file: P<SourceFile>) -> &'static declarationMapSource {
        let text = source_file.original_text();
        P::new(declarationMapSource { file_name: source_file.original_file_name().to_string(), text, line_map: tsrs_core::compute_ecma_line_starts(text) }).get()
    }

    impl tsrs_sourcemap::Source for declarationMapSource {
        // emitter.go:310
        fn file_name(&self) -> &str {
            &self.file_name
        }
        // emitter.go:311
        fn text(&self) -> &str {
            self.text
        }
        // emitter.go:312
        fn ecma_line_map(&self) -> &[tsrs_core::TextPos] {
            &self.line_map
        }
    }

    // emitter.go:93
    fn get_module_transformer(opts: &TransformOptions) -> Option<P<Transformer>> {
        match opts.compiler_options.get_emit_module_kind() {
            ModuleKind::Preserve => {
                // `ESModuleTransformer` contains logic for preserving CJS input syntax in `--module preserve`
                moduletransforms::new_es_module_transformer(opts)
            }
            ModuleKind::ESNext
            | ModuleKind::ES2022
            | ModuleKind::ES2020
            | ModuleKind::ES2015
            | ModuleKind::Node20
            | ModuleKind::Node18
            | ModuleKind::Node16
            | ModuleKind::NodeNext
            | ModuleKind::CommonJS => moduletransforms::new_implied_module_transformer(opts),
            _ => moduletransforms::new_common_js_module_transformer(opts),
        }
    }

    // emitter.go:116
    pub(crate) fn get_script_transformers(emit_context: P<EmitContext>, host: &'static EmitHost, source_file: P<SourceFile>) -> Vec<P<Transformer>> {
        let mut tx: Vec<P<Transformer>> = Vec::new();
        let options = host.options();

        // JS files don't use reference calculations as they don't do import elision, no need to calculate it
        let import_elision_enabled = !options.verbatim_module_syntax.is_true() && !ast::is_in_js_file(Some(source_file.as_node()));
        let jsx_transform_enabled = options.get_jsx_transform_enabled() && source_file.language_variant.get() == LanguageVariant::JSX;

        let emit_resolver = host.get_emit_resolver();

        let reference_resolver = if import_elision_enabled || jsx_transform_enabled || !options.get_isolated_modules() || options.emit_decorator_metadata.is_true() {
            ReferenceResolverRef::Emit(emit_resolver)
        } else {
            ReferenceResolverRef::Plain(tsrs_binder::new_reference_resolver(options, tsrs_binder::ReferenceResolverHooks::<()>::default()))
        };

        let opts = TransformOptions {
            context: emit_context,
            compiler_options: options,
            resolver: reference_resolver,
            emit_resolver: Some(emit_resolver),
            get_emit_module_format_of_file: std::rc::Rc::new(move |file| host.get_emit_module_format_of_file(file)),
        };

        // transform TypeScript syntax
        {
            // use type nodes to add metadata decorators
            if options.emit_decorator_metadata.is_true() {
                tx.extend(tstransforms::new_metadata_transformer(&opts));
            }

            // erase types
            tx.extend(tstransforms::new_type_eraser_transformer(&opts));

            // elide imports
            if import_elision_enabled {
                tx.extend(tstransforms::new_import_elision_transformer(&opts));
            }

            // transform `enum`, `namespace`, and parameter properties
            tx.extend(tstransforms::new_runtime_syntax_transformer(&opts));

            if options.experimental_decorators.is_true() {
                tx.extend(tstransforms::new_legacy_decorators_transformer(&opts));
            }
        }

        if jsx_transform_enabled {
            tx.extend(jsxtransforms::new_jsx_transformer(&opts));
        }

        let downleveler = estransforms::get_es_transformer(&opts);
        if let Some(downleveler) = downleveler {
            tx.push(downleveler);
        }

        tx.extend(estransforms::new_use_strict_transformer(&opts));

        // transform module syntax
        tx.extend(get_module_transformer(&opts));

        // inlining (formerly done via substitutions)
        if !options.get_isolated_modules() {
            tx.extend(inliners::new_const_enum_inlining_transformer(&opts));
        }
        tx
    }

    // emitter.go:444
    pub(crate) fn should_emit_source_maps(map_options: &CompilerOptions, source_file: P<SourceFile>) -> bool {
        (map_options.source_map.is_true() || map_options.inline_source_map.is_true()) && !tspath::file_extension_is(source_file.file_name(), tspath::EXTENSION_JSON)
    }

    // emitter.go:449
    pub(crate) fn get_source_root(map_options: &CompilerOptions) -> String {
        // Normalize source root and make sure it has trailing "/" so that it can be used to combine paths with the
        // relative paths of the sources list in the sourcemap
        let mut source_root = tspath::normalize_slashes(&map_options.source_root);
        if !source_root.is_empty() {
            source_root = tspath::ensure_trailing_directory_separator(&source_root);
        }
        source_root
    }
}
