//! Allocation-light `.tsbuildinfo` decoder.
//!
//! The general JSON decoder retains a complete `Value` tree while the typed `BuildInfo` is allocated. Here serde
//! fills the typed vectors directly. A non-retaining validation pass first preserves the general decoder's strict
//! duplicate-member handling, including in unknown fields which serde intentionally ignores.

use serde::Deserialize;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use std::fmt;
use tsrs_core::collections::OrderedMap;
use tsrs_core::json::Value;

use crate::buildinfo::{
    BuildInfo, BuildInfoDiagnostic, BuildInfoDiagnosticsOfFile, BuildInfoEmitSignature,
    BuildInfoFileInfo, BuildInfoFilePendingEmit, BuildInfoReferenceMapEntry,
    BuildInfoRepopulateInfo, BuildInfoResolvedRoot, BuildInfoRoot, BuildInfoSemanticDiagnostic,
    buildInfoFileInfoNoSignature, buildInfoFileInfoWithSignature, category_from_i32,
    module_kind_from_i32,
};
use crate::snapshot::FileEmitKind;
use tsrs_ast::RepopulateDiagnosticKind;

pub(super) fn unmarshal(text: &str) -> Result<BuildInfo, String> {
    // serde_json accepts duplicate members and keeps the last value in generic/ignored objects. The compiler's JSON
    // decoder rejects them at every depth, so retain that check without retaining another document tree.
    tsrs_core::json::validate(text)?;
    // The existing decoder has no fixed nesting limit. Diagnostic message chains and ignored fields must not gain
    // serde_json's default limit of 128 containers.
    let mut decoder = serde_json::Deserializer::from_str(text);
    decoder.disable_recursion_limit();
    let wire = BuildInfoWire::deserialize(&mut decoder).map_err(|err| err.to_string())?;
    decoder.end().map_err(|err| err.to_string())?;
    wire.build()
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct BuildInfoWire {
    version: Option<String>,
    errors: Option<bool>,
    check_pending: Option<bool>,
    root: Option<Vec<BuildInfoRootWire>>,
    package_jsons: Option<Vec<String>>,
    missing_package_jsons: Option<Vec<String>>,
    content_mapper_identities: Option<Vec<String>>,
    file_names: Option<Vec<String>>,
    file_infos: Option<Vec<BuildInfoFileInfoWire>>,
    file_ids_list: Option<Vec<Option<Vec<i32>>>>,
    options: Option<ObjectWire>,
    referenced_map: Option<Vec<BuildInfoReferenceMapEntryWire>>,
    semantic_diagnostics_per_file: Option<Vec<BuildInfoSemanticDiagnosticWire>>,
    emit_diagnostics_per_file: Option<Vec<BuildInfoDiagnosticsOfFileWire>>,
    change_file_set: Option<Vec<i32>>,
    affected_files_pending_emit: Option<Vec<BuildInfoFilePendingEmitWire>>,
    latest_changed_dts_file: Option<String>,
    emit_signatures: Option<Vec<BuildInfoEmitSignatureWire>>,
    resolved_root: Option<Vec<BuildInfoResolvedRootWire>>,
    semantic_errors: Option<bool>,
}

impl BuildInfoWire {
    fn build(self) -> Result<BuildInfo, String> {
        let file_infos_non_nil = self.file_infos.is_some();
        Ok(BuildInfo {
            version: self.version.unwrap_or_default(),
            errors: self.errors.unwrap_or_default(),
            check_pending: self.check_pending.unwrap_or_default(),
            root: self
                .root
                .unwrap_or_default()
                .into_iter()
                .map(BuildInfoRootWire::build)
                .collect(),
            package_jsons: self.package_jsons,
            missing_package_jsons: self.missing_package_jsons,
            content_mapper_identities: self.content_mapper_identities,
            file_names: self.file_names.unwrap_or_default(),
            file_infos: self
                .file_infos
                .unwrap_or_default()
                .into_iter()
                .map(BuildInfoFileInfoWire::build)
                .collect::<Result<_, _>>()?,
            file_infos_non_nil,
            file_ids_list: self
                .file_ids_list
                .unwrap_or_default()
                .into_iter()
                .map(Option::unwrap_or_default)
                .collect(),
            options: self.options.map(|v| v.0),
            referenced_map: self
                .referenced_map
                .unwrap_or_default()
                .into_iter()
                .map(BuildInfoReferenceMapEntryWire::build)
                .collect(),
            semantic_diagnostics_per_file: self
                .semantic_diagnostics_per_file
                .unwrap_or_default()
                .into_iter()
                .map(BuildInfoSemanticDiagnosticWire::build)
                .collect::<Result<_, _>>()?,
            emit_diagnostics_per_file: self
                .emit_diagnostics_per_file
                .unwrap_or_default()
                .into_iter()
                .map(BuildInfoDiagnosticsOfFileWire::build)
                .collect::<Result<_, _>>()?,
            change_file_set: self.change_file_set.unwrap_or_default(),
            affected_files_pending_emit: self
                .affected_files_pending_emit
                .unwrap_or_default()
                .into_iter()
                .map(BuildInfoFilePendingEmitWire::build)
                .collect::<Result<_, _>>()?,
            latest_changed_dts_file: self.latest_changed_dts_file.unwrap_or_default(),
            emit_signatures: self
                .emit_signatures
                .unwrap_or_default()
                .into_iter()
                .map(BuildInfoEmitSignatureWire::build)
                .collect::<Result<_, _>>()?,
            resolved_root: self
                .resolved_root
                .unwrap_or_default()
                .into_iter()
                .map(BuildInfoResolvedRootWire::build)
                .collect(),
            semantic_errors: self.semantic_errors.unwrap_or_default(),
        })
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BuildInfoRootWire {
    Range(Vec<i32>),
    File(i32),
    NonIncremental(String),
}

impl BuildInfoRootWire {
    fn build(self) -> BuildInfoRoot {
        match self {
            Self::Range(values) => BuildInfoRoot {
                start: values.first().copied().unwrap_or_default(),
                end: values.get(1).copied().unwrap_or_default(),
                ..Default::default()
            },
            Self::File(start) => BuildInfoRoot {
                start,
                ..Default::default()
            },
            Self::NonIncremental(non_incremental) => BuildInfoRoot {
                non_incremental,
                ..Default::default()
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BuildInfoFileInfoWire {
    Signature(String),
    Info(BuildInfoFileInfoObjectWire),
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct BuildInfoFileInfoObjectWire {
    version: Option<String>,
    no_signature: Option<ValueWire>,
    signature: Option<ValueWire>,
    affects_global_scope: Option<bool>,
    implied_node_format: Option<i32>,
}

impl BuildInfoFileInfoWire {
    fn build(self) -> Result<BuildInfoFileInfo, String> {
        match self {
            Self::Signature(signature) => Ok(BuildInfoFileInfo {
                signature,
                ..Default::default()
            }),
            Self::Info(info) => {
                let no_signature = matches!(info.no_signature, Some(ValueWire(Value::Bool(true))));
                if no_signature {
                    return Ok(BuildInfoFileInfo {
                        no_signature: Some(buildInfoFileInfoNoSignature {
                            version: info.version.unwrap_or_default(),
                            no_signature: true,
                            affects_global_scope: info.affects_global_scope.unwrap_or_default(),
                            implied_node_format: module_kind_from_i32(
                                info.implied_node_format.unwrap_or_default(),
                            ),
                        }),
                        ..Default::default()
                    });
                }
                let signature = match info.signature {
                    None | Some(ValueWire(Value::Null)) => String::new(),
                    Some(ValueWire(Value::String(signature))) => signature,
                    Some(_) => return Err("invalid BuildInfoFileInfo".to_string()),
                };
                Ok(BuildInfoFileInfo {
                    file_info: Some(buildInfoFileInfoWithSignature {
                        version: info.version.unwrap_or_default(),
                        signature,
                        affects_global_scope: info.affects_global_scope.unwrap_or_default(),
                        implied_node_format: module_kind_from_i32(
                            info.implied_node_format.unwrap_or_default(),
                        ),
                    }),
                    ..Default::default()
                })
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
struct BuildInfoReferenceMapEntryWire(Vec<i32>);

impl BuildInfoReferenceMapEntryWire {
    fn build(self) -> BuildInfoReferenceMapEntry {
        BuildInfoReferenceMapEntry {
            file_id: self.0.first().copied().unwrap_or_default(),
            file_id_list_id: self.0.get(1).copied().unwrap_or_default(),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct BuildInfoDiagnosticWire {
    file: Option<i32>,
    no_file: Option<bool>,
    pos: Option<i32>,
    end: Option<i32>,
    code: Option<i32>,
    category: Option<i32>,
    source: Option<String>,
    message_text: Option<String>,
    message_key: Option<String>,
    message_args: Option<Vec<String>>,
    message_chain: Option<Vec<BuildInfoDiagnosticWire>>,
    related_information: Option<Vec<BuildInfoDiagnosticWire>>,
    reports_unnecessary: Option<bool>,
    reports_deprecated: Option<bool>,
    skipped_on_no_emit: Option<bool>,
    repopulate_info: Option<BuildInfoRepopulateInfoWire>,
}

impl BuildInfoDiagnosticWire {
    fn build(self) -> Result<BuildInfoDiagnostic, String> {
        Ok(BuildInfoDiagnostic {
            file: self.file.unwrap_or_default(),
            no_file: self.no_file.unwrap_or_default(),
            pos: self.pos.unwrap_or_default(),
            end: self.end.unwrap_or_default(),
            code: self.code.unwrap_or_default(),
            category: category_from_i32(self.category.unwrap_or_default()),
            source: self.source.unwrap_or_default(),
            message_text: self.message_text.unwrap_or_default(),
            message_key: self.message_key.unwrap_or_default(),
            message_args: self.message_args.unwrap_or_default(),
            message_chain: self
                .message_chain
                .unwrap_or_default()
                .into_iter()
                .map(Self::build)
                .collect::<Result<_, _>>()?,
            related_information: self
                .related_information
                .unwrap_or_default()
                .into_iter()
                .map(Self::build)
                .collect::<Result<_, _>>()?,
            reports_unnecessary: self.reports_unnecessary.unwrap_or_default(),
            reports_deprecated: self.reports_deprecated.unwrap_or_default(),
            skipped_on_no_emit: self.skipped_on_no_emit.unwrap_or_default(),
            repopulate_info: self
                .repopulate_info
                .map(BuildInfoRepopulateInfoWire::build)
                .transpose()?,
        })
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct BuildInfoRepopulateInfoWire {
    kind: Option<i32>,
    module_reference: Option<String>,
    mode: Option<i32>,
    package_name: Option<String>,
}

impl BuildInfoRepopulateInfoWire {
    fn build(self) -> Result<BuildInfoRepopulateInfo, String> {
        let kind = match self.kind.unwrap_or_default() {
            1 => RepopulateDiagnosticKind::ModeMismatch,
            2 => RepopulateDiagnosticKind::ModuleNotFound,
            kind => return Err(format!("invalid RepopulateDiagnosticKind: {kind}")),
        };
        Ok(BuildInfoRepopulateInfo {
            kind,
            module_reference: self.module_reference.unwrap_or_default(),
            mode: module_kind_from_i32(self.mode.unwrap_or_default()),
            package_name: self.package_name.unwrap_or_default(),
        })
    }
}

#[derive(Deserialize)]
struct BuildInfoDiagnosticsOfFileWire(i32, Option<Vec<BuildInfoDiagnosticWire>>);

impl BuildInfoDiagnosticsOfFileWire {
    fn build(self) -> Result<BuildInfoDiagnosticsOfFile, String> {
        Ok(BuildInfoDiagnosticsOfFile {
            file_id: self.0,
            diagnostics: self
                .1
                .unwrap_or_default()
                .into_iter()
                .map(BuildInfoDiagnosticWire::build)
                .collect::<Result<_, _>>()?,
        })
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BuildInfoSemanticDiagnosticWire {
    File(i32),
    Diagnostics(BuildInfoDiagnosticsOfFileWire),
}

impl BuildInfoSemanticDiagnosticWire {
    fn build(self) -> Result<BuildInfoSemanticDiagnostic, String> {
        Ok(match self {
            Self::File(file_id) => BuildInfoSemanticDiagnostic {
                file_id,
                diagnostics: None,
            },
            Self::Diagnostics(diagnostics) => BuildInfoSemanticDiagnostic {
                file_id: 0,
                diagnostics: Some(diagnostics.build()?),
            },
        })
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BuildInfoFilePendingEmitWire {
    File(i32),
    FileAndKind(Vec<i32>),
}

impl BuildInfoFilePendingEmitWire {
    fn build(self) -> Result<BuildInfoFilePendingEmit, String> {
        match self {
            Self::File(file_id) => Ok(BuildInfoFilePendingEmit {
                file_id,
                emit_kind: FileEmitKind::None,
            }),
            Self::FileAndKind(values) => match values.as_slice() {
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
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BuildInfoEmitSignatureWire {
    File(i32),
    FileAndSignature((f64, ValueWire)),
}

impl BuildInfoEmitSignatureWire {
    fn build(self) -> Result<BuildInfoEmitSignature, String> {
        match self {
            Self::File(file_id) => Ok(BuildInfoEmitSignature {
                file_id,
                ..Default::default()
            }),
            Self::FileAndSignature((file_id, ValueWire(signature))) => {
                let file_id = file_id as i32;
                match signature {
                    Value::String(signature) => Ok(BuildInfoEmitSignature {
                        file_id,
                        signature,
                        ..Default::default()
                    }),
                    Value::Array(values) if values.is_empty() => Ok(BuildInfoEmitSignature {
                        file_id,
                        differs_only_in_dts_map: true,
                        ..Default::default()
                    }),
                    Value::Array(mut values) if values.len() == 1 => match values.pop().unwrap() {
                        Value::String(signature) => Ok(BuildInfoEmitSignature {
                            file_id,
                            signature,
                            differs_in_options: true,
                            ..Default::default()
                        }),
                        _ => Err(
                            "invalid signature in BuildInfoEmitSignature: expected string"
                                .to_string(),
                        ),
                    },
                    Value::Array(values) => Err(format!(
                        "invalid signature in BuildInfoEmitSignature: expected string or []string with 0 or 1 element, got {} elements",
                        values.len()
                    )),
                    _ => Err(
                        "invalid signature in BuildInfoEmitSignature: expected string or []string"
                            .to_string(),
                    ),
                }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
struct BuildInfoResolvedRootWire(Vec<i32>);

impl BuildInfoResolvedRootWire {
    fn build(self) -> BuildInfoResolvedRoot {
        BuildInfoResolvedRoot {
            resolved: self.0.first().copied().unwrap_or_default(),
            root: self.0.get(1).copied().unwrap_or_default(),
        }
    }
}

struct ObjectWire(OrderedMap<String, Value>);

impl<'de> Deserialize<'de> for ObjectWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = ObjectWire;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object")
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = OrderedMap::default();
                while let Some((key, ValueWire(value))) = map.next_entry::<String, ValueWire>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!(
                            "duplicate object member name {key:?}"
                        )));
                    }
                    values.insert(key, value);
                }
                Ok(ObjectWire(values))
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}

#[derive(Debug)]
struct ValueWire(Value);

impl<'de> Deserialize<'de> for ValueWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ValueVisitor;
        impl<'de> Visitor<'de> for ValueVisitor {
            type Value = ValueWire;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(ValueWire(Value::Null))
            }
            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(ValueWire(Value::Null))
            }
            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(ValueWire(Value::Bool(value)))
            }
            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(ValueWire(Value::Number(value as f64)))
            }
            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(ValueWire(Value::Number(value as f64)))
            }
            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
                Ok(ValueWire(Value::Number(value)))
            }
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ValueWire(Value::String(value.to_string())))
            }
            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(ValueWire(Value::String(value)))
            }
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::with_capacity(seq.size_hint().unwrap_or_default());
                while let Some(ValueWire(value)) = seq.next_element::<ValueWire>()? {
                    values.push(value);
                }
                Ok(ValueWire(Value::Array(values)))
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = OrderedMap::default();
                while let Some((key, ValueWire(value))) = map.next_entry::<String, ValueWire>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!(
                            "duplicate object member name {key:?}"
                        )));
                    }
                    values.insert(key, value);
                }
                Ok(ValueWire(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(ValueVisitor)
    }
}
