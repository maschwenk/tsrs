// Printer-level source-map check: prints an (untransformed) input file with a source-map generator, the way
// compiler/emitter.go printSourceFile does for an output next to the input, and writes `<out>/<base>.js` and
// `<out>/<base>.js.map`. Compare with `tsgo --allowJs --target esnext --module preserve --sourceMap --outDir <out>`
// on a plain `.js` input (the script transformers leave such a file unchanged).
//
//   sourcemap_oracle FILE OUTDIR

use tsrs_ast::{ExternalModuleIndicatorOptions, SourceFileParseOptions};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_printer::{new_printer, new_text_writer, EmitTextWriter, PrintHandlers, PrinterOptions};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (path, out_dir) = (&args[1], &args[2]);
    let text = tsrs_vfs::internal::decode_bytes(std::fs::read(path).unwrap());
    let opts = SourceFileParseOptions { file_name: path.to_string(), path: Path::new(path.to_string()), external_module_indicator_options: ExternalModuleIndicatorOptions { jsx: false, force: false } };
    let file = tsrs_parser::parse_source_file(opts, &text, tsrs_core::ensure_script_kind_from_file_name(path));
    let base = tspath::get_base_file_name(path).to_string();
    let js_file_path = tspath::combine_paths(out_dir, &[&base]);
    let mut generator = tsrs_sourcemap::new_generator(
        &tspath::get_base_file_name(&js_file_path),
        "",
        &tspath::get_directory_path(&tspath::normalize_path(&js_file_path)),
        ComparePathsOptions { use_case_sensitive_file_names: true, current_directory: std::env::current_dir().unwrap().to_string_lossy().into_owned() },
    );
    let mut p = new_printer(PrinterOptions { source_map: true, target: tsrs_core::ScriptTarget::ESNext, ..Default::default() }, PrintHandlers::default(), None);
    let mut writer: Box<dyn EmitTextWriter> = new_text_writer("\n", 0);
    p.write(file.as_node(), Some(file), &mut *writer, Some(&mut generator));
    if !writer.is_at_start_of_line() {
        writer.raw_write("\n");
    }
    writer.write_comment("//# sourceMappingURL=");
    writer.write_comment(&tsrs_core::stringutil::encode_uri(&format!("{}.map", base)));
    std::fs::write(format!("{}.map", js_file_path), generator.string()).unwrap();
    std::fs::write(&js_file_path, writer.string()).unwrap();
}
