//! Single-pass `.tsbuildinfo` decoder.
//!
//! Known fields are decoded directly into their typed representation. Unknown fields are validated and discarded,
//! and only the open-ended compiler `options` object becomes a generic JSON tree.

use tsrs_ast::RepopulateDiagnosticKind;
use tsrs_core::json::{self, Decoder, Value, ValueKind};

use crate::buildinfo::{
    BuildInfo, BuildInfoDiagnostic, BuildInfoDiagnosticsOfFile, BuildInfoEmitSignature,
    BuildInfoFileInfo, BuildInfoFilePendingEmit, BuildInfoReferenceMapEntry,
    BuildInfoRepopulateInfo, BuildInfoResolvedRoot, BuildInfoRoot, BuildInfoSemanticDiagnostic,
    buildInfoFileInfoNoSignature, buildInfoFileInfoWithSignature, category_from_i32,
    module_kind_from_i32,
};
use crate::snapshot::FileEmitKind;

pub(super) fn unmarshal(text: &str) -> Result<BuildInfo, String> {
    json::decode(text, |decoder| {
        let mut build_info = BuildInfo::default();
        decoder.read_object(|decoder, key| {
            match key {
                "version" => build_info.version = read_string(decoder)?,
                "errors" => build_info.errors = read_bool(decoder)?,
                "checkPending" => build_info.check_pending = read_bool(decoder)?,
                "root" => build_info.root = read_array(decoder, read_root)?,
                "packageJsons" => build_info.package_jsons = read_optional_strings(decoder)?,
                "missingPackageJsons" => {
                    build_info.missing_package_jsons = read_optional_strings(decoder)?
                }
                "contentMapperIdentities" => {
                    build_info.content_mapper_identities = read_optional_strings(decoder)?
                }
                "fileNames" => {
                    build_info.file_names = read_array(decoder, |decoder| decoder.read_string())?
                }
                "fileInfos" => {
                    if decoder.read_null()? {
                        build_info.file_infos.clear();
                        build_info.file_infos_non_nil = false;
                    } else {
                        build_info.file_infos = read_array_nonnull(decoder, read_file_info)?;
                        build_info.file_infos_non_nil = true;
                    }
                }
                "fileIdsList" => {
                    build_info.file_ids_list = read_array(decoder, |decoder| {
                        read_array(decoder, |decoder| decoder.read_i32())
                    })?;
                }
                "options" => {
                    build_info.options = if decoder.read_null()? {
                        None
                    } else {
                        match decoder.read_value()? {
                            Value::Object(options) => Some(options),
                            _ => return Err("invalid options".to_string()),
                        }
                    };
                }
                "referencedMap" => {
                    build_info.referenced_map = read_array(decoder, read_reference_map_entry)?
                }
                "semanticDiagnosticsPerFile" => {
                    build_info.semantic_diagnostics_per_file =
                        read_array(decoder, read_semantic_diagnostic)?;
                }
                "emitDiagnosticsPerFile" => {
                    build_info.emit_diagnostics_per_file =
                        read_array(decoder, read_diagnostics_of_file)?;
                }
                "changeFileSet" => {
                    build_info.change_file_set = read_array(decoder, |decoder| decoder.read_i32())?
                }
                "affectedFilesPendingEmit" => {
                    build_info.affected_files_pending_emit =
                        read_array(decoder, read_file_pending_emit)?;
                }
                "latestChangedDtsFile" => {
                    build_info.latest_changed_dts_file = read_string(decoder)?
                }
                "emitSignatures" => {
                    build_info.emit_signatures = read_array(decoder, read_emit_signature)?
                }
                "resolvedRoot" => {
                    build_info.resolved_root = read_array(decoder, read_resolved_root)?
                }
                "semanticErrors" => build_info.semantic_errors = read_bool(decoder)?,
                _ => decoder.skip_value()?,
            }
            Ok(())
        })?;
        Ok(build_info)
    })
}

fn read_string(decoder: &mut Decoder<'_>) -> Result<String, String> {
    if decoder.read_null()? {
        Ok(String::new())
    } else {
        decoder.read_string()
    }
}

fn read_bool(decoder: &mut Decoder<'_>) -> Result<bool, String> {
    if decoder.read_null()? {
        Ok(false)
    } else {
        decoder.read_bool()
    }
}

fn read_i32(decoder: &mut Decoder<'_>) -> Result<i32, String> {
    if decoder.read_null()? {
        Ok(0)
    } else {
        decoder.read_i32()
    }
}

fn read_array<T>(
    decoder: &mut Decoder<'_>,
    element: impl FnMut(&mut Decoder<'_>) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    if decoder.read_null()? {
        Ok(Vec::new())
    } else {
        read_array_nonnull(decoder, element)
    }
}

fn read_array_nonnull<T>(
    decoder: &mut Decoder<'_>,
    mut element: impl FnMut(&mut Decoder<'_>) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    let mut values = Vec::new();
    decoder.read_array(|decoder| {
        values.push(element(decoder)?);
        Ok(())
    })?;
    Ok(values)
}

fn read_optional_strings(decoder: &mut Decoder<'_>) -> Result<Option<Vec<String>>, String> {
    if decoder.read_null()? {
        Ok(None)
    } else {
        read_array_nonnull(decoder, |decoder| decoder.read_string()).map(Some)
    }
}

fn read_root(decoder: &mut Decoder<'_>) -> Result<BuildInfoRoot, String> {
    match decoder.kind()? {
        ValueKind::Array => {
            let values = read_array_nonnull(decoder, |decoder| decoder.read_i32())?;
            Ok(BuildInfoRoot {
                start: values.first().copied().unwrap_or_default(),
                end: values.get(1).copied().unwrap_or_default(),
                ..Default::default()
            })
        }
        ValueKind::Number => Ok(BuildInfoRoot {
            start: decoder.read_i32()?,
            ..Default::default()
        }),
        ValueKind::String => Ok(BuildInfoRoot {
            non_incremental: decoder.read_string()?,
            ..Default::default()
        }),
        _ => Err("invalid BuildInfoRoot".to_string()),
    }
}

enum StringField {
    Missing,
    String(String),
    Invalid,
}

fn read_file_info(decoder: &mut Decoder<'_>) -> Result<BuildInfoFileInfo, String> {
    if decoder.kind()? == ValueKind::String {
        return Ok(BuildInfoFileInfo {
            signature: decoder.read_string()?,
            ..Default::default()
        });
    }
    let mut version = String::new();
    let mut no_signature = false;
    let mut signature = StringField::Missing;
    let mut affects_global_scope = false;
    let mut implied_node_format = 0;
    decoder.read_object(|decoder, key| {
        match key {
            "version" => version = read_string(decoder)?,
            "noSignature" => {
                no_signature = match decoder.kind()? {
                    ValueKind::Bool => decoder.read_bool()?,
                    ValueKind::Null => {
                        decoder.read_null()?;
                        false
                    }
                    _ => {
                        decoder.skip_value()?;
                        false
                    }
                };
            }
            "signature" => {
                signature = match decoder.kind()? {
                    ValueKind::String => StringField::String(decoder.read_string()?),
                    ValueKind::Null => {
                        decoder.read_null()?;
                        StringField::Missing
                    }
                    _ => {
                        decoder.skip_value()?;
                        StringField::Invalid
                    }
                };
            }
            "affectsGlobalScope" => affects_global_scope = read_bool(decoder)?,
            "impliedNodeFormat" => implied_node_format = read_i32(decoder)?,
            _ => decoder.skip_value()?,
        }
        Ok(())
    })?;
    if no_signature {
        return Ok(BuildInfoFileInfo {
            no_signature: Some(buildInfoFileInfoNoSignature {
                version,
                no_signature: true,
                affects_global_scope,
                implied_node_format: module_kind_from_i32(implied_node_format),
            }),
            ..Default::default()
        });
    }
    let signature = match signature {
        StringField::Missing => String::new(),
        StringField::String(signature) => signature,
        StringField::Invalid => return Err("invalid BuildInfoFileInfo".to_string()),
    };
    Ok(BuildInfoFileInfo {
        file_info: Some(buildInfoFileInfoWithSignature {
            version,
            signature,
            affects_global_scope,
            implied_node_format: module_kind_from_i32(implied_node_format),
        }),
        ..Default::default()
    })
}

fn read_reference_map_entry(
    decoder: &mut Decoder<'_>,
) -> Result<BuildInfoReferenceMapEntry, String> {
    let values = read_array_nonnull(decoder, |decoder| decoder.read_i32())?;
    Ok(BuildInfoReferenceMapEntry {
        file_id: values.first().copied().unwrap_or_default(),
        file_id_list_id: values.get(1).copied().unwrap_or_default(),
    })
}

fn read_diagnostic(decoder: &mut Decoder<'_>) -> Result<BuildInfoDiagnostic, String> {
    let mut diagnostic = BuildInfoDiagnostic::default();
    decoder.read_object(|decoder, key| {
        match key {
            "file" => diagnostic.file = read_i32(decoder)?,
            "noFile" => diagnostic.no_file = read_bool(decoder)?,
            "pos" => diagnostic.pos = read_i32(decoder)?,
            "end" => diagnostic.end = read_i32(decoder)?,
            "code" => diagnostic.code = read_i32(decoder)?,
            "category" => diagnostic.category = category_from_i32(read_i32(decoder)?),
            "source" => diagnostic.source = read_string(decoder)?,
            "messageText" => diagnostic.message_text = read_string(decoder)?,
            "messageKey" => diagnostic.message_key = read_string(decoder)?,
            "messageArgs" => {
                diagnostic.message_args = read_array(decoder, |decoder| decoder.read_string())?
            }
            "messageChain" => diagnostic.message_chain = read_array(decoder, read_diagnostic)?,
            "relatedInformation" => {
                diagnostic.related_information = read_array(decoder, read_diagnostic)?
            }
            "reportsUnnecessary" => diagnostic.reports_unnecessary = read_bool(decoder)?,
            "reportsDeprecated" => diagnostic.reports_deprecated = read_bool(decoder)?,
            "skippedOnNoEmit" => diagnostic.skipped_on_no_emit = read_bool(decoder)?,
            "repopulateInfo" => {
                diagnostic.repopulate_info = if decoder.read_null()? {
                    None
                } else {
                    Some(read_repopulate_info(decoder)?)
                };
            }
            _ => decoder.skip_value()?,
        }
        Ok(())
    })?;
    Ok(diagnostic)
}

fn read_repopulate_info(decoder: &mut Decoder<'_>) -> Result<BuildInfoRepopulateInfo, String> {
    let mut kind = 0;
    let mut module_reference = String::new();
    let mut mode = 0;
    let mut package_name = String::new();
    decoder.read_object(|decoder, key| {
        match key {
            "kind" => kind = read_i32(decoder)?,
            "moduleReference" => module_reference = read_string(decoder)?,
            "mode" => mode = read_i32(decoder)?,
            "packageName" => package_name = read_string(decoder)?,
            _ => decoder.skip_value()?,
        }
        Ok(())
    })?;
    let kind = match kind {
        1 => RepopulateDiagnosticKind::ModeMismatch,
        2 => RepopulateDiagnosticKind::ModuleNotFound,
        kind => return Err(format!("invalid RepopulateDiagnosticKind: {kind}")),
    };
    Ok(BuildInfoRepopulateInfo {
        kind,
        module_reference,
        mode: module_kind_from_i32(mode),
        package_name,
    })
}

fn read_diagnostics_of_file(
    decoder: &mut Decoder<'_>,
) -> Result<BuildInfoDiagnosticsOfFile, String> {
    let mut file_id = 0;
    let mut diagnostics = Vec::new();
    let mut count = 0;
    decoder.read_array(|decoder| {
        match count {
            0 => file_id = decoder.read_i32()?,
            1 => diagnostics = read_array(decoder, read_diagnostic)?,
            _ => decoder.skip_value()?,
        }
        count += 1;
        Ok(())
    })?;
    if count != 2 {
        return Err(format!(
            "invalid BuildInfoDiagnosticsOfFile: expected 2 elements, got {count}"
        ));
    }
    Ok(BuildInfoDiagnosticsOfFile {
        file_id,
        diagnostics,
    })
}

fn read_semantic_diagnostic(
    decoder: &mut Decoder<'_>,
) -> Result<BuildInfoSemanticDiagnostic, String> {
    match decoder.kind()? {
        ValueKind::Number => Ok(BuildInfoSemanticDiagnostic {
            file_id: decoder.read_i32()?,
            diagnostics: None,
        }),
        ValueKind::Array => Ok(BuildInfoSemanticDiagnostic {
            file_id: 0,
            diagnostics: Some(read_diagnostics_of_file(decoder)?),
        }),
        _ => Err("invalid BuildInfoSemanticDiagnostic".to_string()),
    }
}

fn read_file_pending_emit(decoder: &mut Decoder<'_>) -> Result<BuildInfoFilePendingEmit, String> {
    if decoder.kind()? == ValueKind::Number {
        return Ok(BuildInfoFilePendingEmit {
            file_id: decoder.read_i32()?,
            emit_kind: FileEmitKind::None,
        });
    }
    let values = read_array_nonnull(decoder, |decoder| decoder.read_i32())?;
    match values.as_slice() {
        [file_id] => Ok(BuildInfoFilePendingEmit {
            file_id: *file_id,
            emit_kind: FileEmitKind::Dts,
        }),
        [file_id, emit_kind] => Ok(BuildInfoFilePendingEmit {
            file_id: *file_id,
            emit_kind: FileEmitKind::from_bits_retain(*emit_kind as u32),
        }),
        [] => Err("invalid BuildInfoFilePendingEmit".to_string()),
        _ => Err(format!(
            "invalid BuildInfoFilePendingEmit: expected 1 or 2 integers, got {}",
            values.len()
        )),
    }
}

enum EmitSignatureValue {
    Missing,
    String(String),
    List(Vec<String>),
    Invalid,
}

fn read_emit_signature(decoder: &mut Decoder<'_>) -> Result<BuildInfoEmitSignature, String> {
    if decoder.kind()? == ValueKind::Number {
        return Ok(BuildInfoEmitSignature {
            file_id: decoder.read_i32()?,
            ..Default::default()
        });
    }
    let mut file_id = 0.0;
    let mut signature = EmitSignatureValue::Missing;
    let mut count = 0;
    decoder.read_array(|decoder| {
        match count {
            0 => file_id = decoder.read_f64()?,
            1 => {
                signature = match decoder.kind()? {
                    ValueKind::String => EmitSignatureValue::String(decoder.read_string()?),
                    ValueKind::Array => {
                        EmitSignatureValue::List(read_array_nonnull(decoder, |decoder| {
                            decoder.read_string()
                        })?)
                    }
                    _ => {
                        decoder.skip_value()?;
                        EmitSignatureValue::Invalid
                    }
                };
            }
            _ => decoder.skip_value()?,
        }
        count += 1;
        Ok(())
    })?;
    if count != 2 {
        return Err(format!(
            "invalid BuildInfoEmitSignature: expected 2 elements, got {count}"
        ));
    }
    let file_id = file_id as i32;
    match signature {
        EmitSignatureValue::String(signature) => Ok(BuildInfoEmitSignature {
            file_id,
            signature,
            ..Default::default()
        }),
        EmitSignatureValue::List(signatures) if signatures.is_empty() => {
            Ok(BuildInfoEmitSignature {
                file_id,
                differs_only_in_dts_map: true,
                ..Default::default()
            })
        }
        EmitSignatureValue::List(mut signatures) if signatures.len() == 1 => {
            Ok(BuildInfoEmitSignature {
                file_id,
                signature: signatures.pop().unwrap(),
                differs_in_options: true,
                ..Default::default()
            })
        }
        EmitSignatureValue::List(signatures) => Err(format!(
            "invalid signature in BuildInfoEmitSignature: expected string or []string with 0 or 1 element, got {} elements",
            signatures.len()
        )),
        EmitSignatureValue::Missing | EmitSignatureValue::Invalid => Err(
            "invalid signature in BuildInfoEmitSignature: expected string or []string".to_string(),
        ),
    }
}

fn read_resolved_root(decoder: &mut Decoder<'_>) -> Result<BuildInfoResolvedRoot, String> {
    let values = read_array_nonnull(decoder, |decoder| decoder.read_i32())?;
    Ok(BuildInfoResolvedRoot {
        resolved: values.first().copied().unwrap_or_default(),
        root: values.get(1).copied().unwrap_or_default(),
    })
}
