// Port of execute/tsctests/readablebuildinfo.go (only the marshaling direction is used by the harness).

use tsrs_core::collections::OrderedMap;
use tsrs_core::json::Value;
use tsrs_incremental::{get_file_emit_kind, BuildInfo, BuildInfoDiagnostic, BuildInfoDiagnosticsOfFile, BuildInfoFileId, FileEmitKind};

#[derive(Default)]
struct obj(OrderedMap<String, Value>);

impl obj {
    fn set(&mut self, k: &str, v: Value) {
        self.0.insert(k.to_string(), v);
    }
    fn str(&mut self, k: &str, v: &str) {
        if !v.is_empty() {
            self.set(k, Value::String(v.to_string()));
        }
    }
    fn bool(&mut self, k: &str, v: bool) {
        if v {
            self.set(k, Value::Bool(true));
        }
    }
    fn int(&mut self, k: &str, v: i64) {
        if v != 0 {
            self.set(k, Value::Number(v as f64));
        }
    }
    fn list(&mut self, k: &str, v: Option<Vec<Value>>) {
        if let Some(v) = v {
            self.set(k, Value::Array(v));
        }
    }
    fn done(self) -> Value {
        Value::Object(self.0)
    }
}

fn strings(v: &[String]) -> Value {
    Value::Array(v.iter().map(|s| Value::String(s.clone())).collect())
}

// Go core.Map over a slice: nil stays nil (omitted by omitzero), empty non-nil stays [].
fn map_nonempty<T>(v: &[T], f: impl Fn(&T) -> Value) -> Option<Vec<Value>> {
    if v.is_empty() {
        None
    } else {
        Some(v.iter().map(f).collect())
    }
}

struct readable<'a> {
    build_info: &'a BuildInfo,
    file_ids_list: Vec<Vec<String>>,
}

impl readable<'_> {
    fn to_file_path(&self, file_id: BuildInfoFileId) -> String {
        self.build_info.file_names[file_id as usize - 1].clone()
    }

    fn diagnostics(&self, diagnostics: &[BuildInfoDiagnostic]) -> Vec<Value> {
        diagnostics
            .iter()
            .map(|d| {
                let mut o = obj::default();
                if d.file != 0 {
                    o.str("file", &self.to_file_path(d.file));
                }
                o.bool("noFile", d.no_file);
                o.int("pos", d.pos as i64);
                o.int("end", d.end as i64);
                o.int("code", d.code as i64);
                o.int("category", tsrs_incremental::category_to_i32(d.category) as i64);
                o.str("messageKey", &d.message_key);
                if !d.message_args.is_empty() {
                    o.set("messageArgs", strings(&d.message_args));
                }
                o.list("messageChain", map_nonempty(&d.message_chain, |_| Value::Null).map(|_| self.diagnostics(&d.message_chain)));
                o.list("relatedInformation", map_nonempty(&d.related_information, |_| Value::Null).map(|_| self.diagnostics(&d.related_information)));
                o.bool("reportsUnnecessary", d.reports_unnecessary);
                o.bool("reportsDeprecated", d.reports_deprecated);
                o.bool("skippedOnNoEmit", d.skipped_on_no_emit);
                if let Some(info) = &d.repopulate_info {
                    let mut r = obj::default();
                    r.set("kind", Value::Number(info.kind as i32 as f64));
                    r.str("moduleReference", &info.module_reference);
                    r.int("mode", info.mode.value() as i64);
                    r.str("packageName", &info.package_name);
                    o.set("repopulateInfo", r.done());
                }
                o.done()
            })
            .collect()
    }

    fn diagnostics_of_file(&self, d: &BuildInfoDiagnosticsOfFile) -> Value {
        Value::Array(vec![Value::String(self.to_file_path(d.file_id)), Value::Array(self.diagnostics(&d.diagnostics))])
    }
}

// readablebuildinfo.go:375
pub(crate) fn to_readable_file_emit_kind(file_emit_kind: FileEmitKind) -> String {
    let mut builder = String::new();
    let mut add_flags = |flags: &str| {
        if !builder.is_empty() {
            builder.push('|');
        }
        builder.push_str(flags);
    };
    if !file_emit_kind.is_empty() {
        if file_emit_kind.intersects(FileEmitKind::Js) {
            add_flags("Js");
        }
        if file_emit_kind.intersects(FileEmitKind::JsMap) {
            add_flags("JsMap");
        }
        if file_emit_kind.intersects(FileEmitKind::JsInlineMap) {
            add_flags("JsInlineMap");
        }
        if file_emit_kind.contains(FileEmitKind::Dts) {
            add_flags("Dts");
        } else {
            if file_emit_kind.intersects(FileEmitKind::DtsEmit) {
                add_flags("DtsEmit");
            }
            if file_emit_kind.intersects(FileEmitKind::DtsErrors) {
                add_flags("DtsErrors");
            }
        }
        if file_emit_kind.intersects(FileEmitKind::DtsMap) {
            add_flags("DtsMap");
        }
    }
    if !builder.is_empty() {
        return builder;
    }
    "None".to_string()
}

// readablebuildinfo.go:240
pub(crate) fn to_readable_build_info(build_info: &BuildInfo, build_info_text: &str) -> String {
    let file_ids_list: Vec<Vec<String>> =
        build_info.file_ids_list.iter().map(|ids| ids.iter().map(|&id| build_info.file_names[id as usize - 1].clone()).collect()).collect();
    let r = readable { build_info, file_ids_list };
    let mut o = obj::default();
    o.str("version", &build_info.version);
    o.bool("errors", build_info.errors);
    o.bool("checkPending", build_info.check_pending);
    // setRoot
    o.list(
        "root",
        map_nonempty(&build_info.root, |original| {
            let files: Vec<String> = if !original.non_incremental.is_empty() {
                vec![original.non_incremental.clone()]
            } else if original.end == 0 {
                vec![r.to_file_path(original.start)]
            } else {
                (original.start..=original.end).map(|i| r.to_file_path(i)).collect()
            };
            let mut ro = obj::default();
            if !files.is_empty() {
                ro.set("files", strings(&files));
            }
            ro.set("original", original.marshal_json());
            ro.done()
        }),
    );
    if let Some(p) = build_info.package_jsons.as_ref().filter(|p| !p.is_empty()) {
        o.set("packageJsons", strings(p));
    }
    if let Some(p) = build_info.missing_package_jsons.as_ref().filter(|p| !p.is_empty()) {
        o.set("missingPackageJsons", strings(p));
    }
    if !build_info.file_names.is_empty() {
        o.set("fileNames", strings(&build_info.file_names));
    }
    // setFileInfos
    if build_info.file_infos_non_nil || !build_info.file_infos.is_empty() {
        let infos: Vec<Value> = build_info
            .file_infos
            .iter()
            .enumerate()
            .map(|(index, original)| {
                let file_info = original.get_file_info();
                let mut fo = obj::default();
                fo.str("fileName", &r.to_file_path(index as BuildInfoFileId + 1));
                fo.str("version", file_info.version());
                fo.str("signature", file_info.signature());
                fo.bool("affectsGlobalScope", file_info.affects_global_scope());
                fo.str("impliedNodeFormat", &format!("{:?}", file_info.implied_node_format()));
                // Dont set original for string encoding
                if !original.has_signature() {
                    fo.set("original", original.marshal_json());
                }
                fo.done()
            })
            .collect();
        o.set("fileInfos", Value::Array(infos));
    }
    o.list("fileIdsList", map_nonempty(&r.file_ids_list, |ids| strings(ids)));
    if let Some(options) = &build_info.options {
        o.set("options", Value::Object(options.clone()));
    }
    if !build_info.referenced_map.is_empty() {
        let mut m = OrderedMap::default();
        for entry in &build_info.referenced_map {
            m.insert(r.to_file_path(entry.file_id), strings(&r.file_ids_list[entry.file_id_list_id as usize - 1]));
        }
        o.set("referencedMap", Value::Object(m));
    }
    o.list(
        "semanticDiagnosticsPerFile",
        map_nonempty(&build_info.semantic_diagnostics_per_file, |d| {
            if d.file_id != 0 {
                Value::String(r.to_file_path(d.file_id))
            } else {
                r.diagnostics_of_file(d.diagnostics.as_ref().unwrap())
            }
        }),
    );
    o.list("emitDiagnosticsPerFile", map_nonempty(&build_info.emit_diagnostics_per_file, |d| r.diagnostics_of_file(d)));
    o.list("changeFileSet", map_nonempty(&build_info.change_file_set, |&id| Value::String(r.to_file_path(id))));
    if !build_info.affected_files_pending_emit.is_empty() {
        let full_emit_kind = get_file_emit_kind(&build_info.get_compiler_options(""));
        o.set(
            "affectedFilesPendingEmit",
            Value::Array(
                build_info
                    .affected_files_pending_emit
                    .iter()
                    .map(|pending_emit| {
                        let emit_kind = if pending_emit.emit_kind.is_empty() { full_emit_kind } else { pending_emit.emit_kind };
                        Value::Array(vec![
                            Value::String(r.to_file_path(pending_emit.file_id)),
                            Value::String(to_readable_file_emit_kind(emit_kind)),
                            pending_emit.marshal_json(),
                        ])
                    })
                    .collect(),
            ),
        );
    }
    o.str("latestChangedDtsFile", &build_info.latest_changed_dts_file);
    o.list(
        "emitSignatures",
        map_nonempty(&build_info.emit_signatures, |signature| {
            let mut so = obj::default();
            so.str("file", &r.to_file_path(signature.file_id));
            so.str("signature", &signature.signature);
            so.bool("differsOnlyInDtsMap", signature.differs_only_in_dts_map);
            so.bool("differsInOptions", signature.differs_in_options);
            so.set("original", signature.marshal_json());
            so.done()
        }),
    );
    o.list(
        "resolvedRoot",
        map_nonempty(&build_info.resolved_root, |original| {
            Value::Array(vec![Value::String(r.to_file_path(original.resolved)), Value::String(r.to_file_path(original.root))])
        }),
    );
    o.int("size", build_info_text.len() as i64);
    o.bool("semanticErrors", build_info.semantic_errors);
    tsrs_core::json::marshal_indent(&o.done(), "", "  ").unwrap()
}
