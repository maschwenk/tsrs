// session.go source-file handling: encodeSourceFileResponse, newSourceFileDescriptor, sourceFileNodeID,
// nodeHandleFrom / resolveNodeHandle, source-file leases (createSourceFile*, retainSourceFile,
// releaseSourceFile, getCachedSourceFile) and getSourceFile / getConfigSourceFile.
//
// Node index tables: Go caches one table per AST on the source file itself (`GetOrComputeData`), created
// either by the first encode or by the first node-handle request. tsrs source files have no per-file data
// slot, so the session keeps the cache keyed by the file's address. Looking a table up must not assign the
// file's lazy node id (that would change `NodeIndexTable::get_index` order relative to Go). An entry is
// removed when the region owning the file is freed (`Region::on_free` runs before the memory is reused), so a
// table never outlives the nodes it points to; files outside any region live for the process.

use rustc_hash::FxHashMap;
use std::sync::{Arc, Mutex};

use tsrs_api_codec::{self as codec, NodeIndexTable};
use tsrs_ast::{Node, SourceFile, SourceFileParseOptions};
use tsrs_compiler::Program;
use tsrs_core::json::Value;
use tsrs_core::{tspath, ScriptKind, P};
use tsrs_project::SourceFileLease;

use crate::batch::base64_encode;
use crate::handler::{ApiError, ApiResult, Response};
use crate::session::{json_response, Session};
use crate::wire::{s, Obj, Params};

#[derive(Default)]
pub(crate) struct SourceFileState {
    tables: Mutex<FxHashMap<usize, Arc<NodeIndexTable>>>,
    leases: Mutex<FxHashMap<u64, Arc<SourceFileLease>>>,
    next_lease: std::sync::atomic::AtomicU64,
}

impl SourceFileState {
    pub(crate) fn release_all_leases(&self) {
        let leases: Vec<_> = self.leases.lock().unwrap().drain().map(|(_, l)| l).collect();
        for l in leases {
            l.release();
        }
    }
}

/// Go `sourceFileNodeID`.
pub fn source_file_node_id(file: P<SourceFile>) -> u64 {
    tsrs_ast::get_node_id(file.as_node()).0
}

fn script_kind_from(n: u64) -> Option<ScriptKind> {
    Some(match n {
        0 => ScriptKind::Unknown,
        1 => ScriptKind::JS,
        2 => ScriptKind::JSX,
        3 => ScriptKind::TS,
        4 => ScriptKind::TSX,
        6 => ScriptKind::JSON,
        _ => return None,
    })
}

fn is_valid_create_script_kind(k: ScriptKind) -> bool {
    matches!(k, ScriptKind::JS | ScriptKind::JSX | ScriptKind::TS | ScriptKind::TSX | ScriptKind::JSON)
}

/// Go `newSourceFileDescriptor` as the wire object.
pub fn source_file_descriptor(file: P<SourceFile>) -> Value {
    let o = file.parse_options();
    let key = u32::from(o.external_module_indicator_options.jsx) | (u32::from(o.external_module_indicator_options.force) << 1);
    Obj::new()
        .set("fileName", s(o.file_name.clone()))
        .set("path", s(o.path.as_str()))
        .set("contentHash", s(codec::source_file_hash(&file)))
        .set("parseOptionsKey", s(key.to_string()))
        .set("scriptKind", Value::Number(file.script_kind() as i32 as f64))
        .set("nodeId", s(source_file_node_id(file).to_string()))
        .build()
}

/// Go `SourceFileDescriptor.parseCacheKey`.
fn parse_cache_key(d: &Value) -> Result<tsrs_project::ParseCacheKey, String> {
    let p = Params(d);
    let hash = p.str("contentHash").map_err(|e| e.message)?;
    if hash.len() != 32 {
        return Err("content hash must contain 32 hexadecimal digits".to_string());
    }
    let hi = u64::from_str_radix(&hash[..16], 16).map_err(|e| format!("invalid content hash: {e}"))?;
    let lo = u64::from_str_radix(&hash[16..], 16).map_err(|e| format!("invalid content hash: {e}"))?;
    let key_text = p.str("parseOptionsKey").map_err(|e| e.message)?;
    let key: u32 = key_text.parse().ok().filter(|k| k & !3 == 0).ok_or_else(|| format!("invalid parse options key {key_text:?}"))?;
    let sk = match p.get("scriptKind") {
        Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 => script_kind_from(*n as u64),
        _ => None,
    };
    let sk = sk.filter(|k| is_valid_create_script_kind(*k)).ok_or_else(|| format!("invalid script kind {}", tsrs_core::json::marshal(p.get("scriptKind")).unwrap_or_default()))?;
    let options = SourceFileParseOptions {
        file_name: p.str("fileName").map_err(|e| e.message)?.to_string(),
        path: tspath::Path::new(p.str("path").map_err(|e| e.message)?),
        external_module_indicator_options: tsrs_ast::ExternalModuleIndicatorOptions { jsx: key & 1 != 0, force: key & 2 != 0 },
    };
    Ok(tsrs_project::new_parse_cache_key(options, ((hi as u128) << 64) | lo as u128, sk))
}

impl Session {
    /// Go `encoder.GetNodeIndexTable` (cached per live file; see the module comment).
    pub(crate) fn node_index_table(&self, file: P<SourceFile>) -> Arc<NodeIndexTable> {
        let addr = file.as_node().addr();
        if let Some(t) = self.source_files.tables.lock().unwrap().get(&addr) {
            return Arc::clone(t);
        }
        // SAFETY: `file` is reachable from a live snapshot or lease for the whole request, and the cached table built
        // from it is evicted when the file's arena region is freed (`store_table`).
        let file_ref: &'static SourceFile = unsafe { &*(&raw const *file) };
        self.store_table(addr, Arc::new(codec::build_node_index_table(file_ref)))
    }

    /// Go `GetOrComputeData`: keeps an existing table for the file, else stores `table`.
    fn store_table(&self, addr: usize, table: Arc<NodeIndexTable>) -> Arc<NodeIndexTable> {
        let mut tables = self.source_files.tables.lock().unwrap();
        if let Some(t) = tables.get(&addr) {
            return Arc::clone(t);
        }
        tables.insert(addr, Arc::clone(&table));
        drop(tables);
        if let Some(region) = tsrs_core::arena::Region::containing(addr) {
            let weak = self.weak_self();
            region.on_free(Box::new(move || {
                if let Some(s) = weak.upgrade() {
                    s.source_files.tables.lock().unwrap().remove(&addr);
                }
            }));
        }
        table
    }

    /// Go `encoder.EncodeSourceFile` + `SetSourceFileID`.
    fn encode_source_file(&self, file: P<SourceFile>) -> ApiResult<Vec<u8>> {
        // SAFETY: as in `node_index_table`: the file outlives the request, and the cached table is evicted when its arena
        // region is freed.
        let file_ref: &'static SourceFile = unsafe { &*(&raw const *file) };
        let (mut data, table) = codec::encode_source_file(file_ref).map_err(|e| ApiError::internal(format!("failed to encode source file: {e}")))?;
        self.store_table(file.as_node().addr(), Arc::new(table));
        // Go `SetSourceFileID(data, sourceFileNodeID(file))` after encoding.
        codec::set_source_file_id(&mut data, source_file_node_id(file));
        Ok(data)
    }

    fn binary_or_base64(&self, data: Vec<u8>) -> ApiResult<Response> {
        if self.binary_responses() {
            Ok(Response::Binary(data))
        } else {
            json_response(&Obj::new().set("data", s(base64_encode(&data))).build())
        }
    }

    /// Go `encodeSourceFileResponse`.
    pub(crate) fn encode_source_file_response(&self, file: Option<P<SourceFile>>) -> ApiResult<Response> {
        match file {
            None if self.binary_responses() => Ok(Response::Binary(Vec::new())),
            None => Ok(Response::null()),
            Some(f) => {
                let data = self.encode_source_file(f)?;
                self.binary_or_base64(data)
            }
        }
    }

    /// Go `nodeHandleFrom`.
    pub(crate) fn node_handle_from(&self, node: P<Node>) -> ApiResult<String> {
        let file = tsrs_ast::get_source_file_of_node(node).ok_or_else(|| ApiError::internal("node has no source file"))?;
        let table = self.node_index_table(file);
        Ok(codec::node_handle(&table, &file, node))
    }

    /// Go `snapshotData.resolveNodeHandle`.
    pub(crate) fn resolve_node_handle(&self, program: &'static Program, handle: &str) -> ApiResult<P<Node>> {
        let parsed = codec::parse_node_handle(handle).ok_or_else(|| ApiError::client(format!("invalid node handle {handle:?}")))?;
        let file = program
            .get_source_file_by_path(&tspath::Path::new(parsed.path))
            .ok_or_else(|| ApiError::client(format!("node handle {handle:?} could not be resolved (file may not be loaded or handle may be stale)")))?;
        let table = self.node_index_table(file);
        codec::resolve_node_index(&table, parsed.index)
            .ok_or_else(|| ApiError::client(format!("node handle {handle:?} could not be resolved (file may not be loaded or handle may be stale)")))
    }

    /// Go `encoder.EncodeNode(node, nil)`.
    pub(crate) fn encode_node(&self, node: P<Node>) -> ApiResult<Vec<u8>> {
        codec::encode_node(node, None).map(|(d, _)| d).map_err(|e| ApiError::internal(format!("failed to encode node: {e}")))
    }

    fn register_lease(&self, lease: SourceFileLease) -> u64 {
        let id = self.source_files.next_lease.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.source_files.leases.lock().unwrap().insert(id, Arc::new(lease));
        id
    }

    /// Go `acquireCachedSourceFile`.
    pub(crate) fn acquire_cached_source_file(&self, descriptor: &Value) -> ApiResult<SourceFileLease> {
        // Field types are part of decoding the request (Go json): mismatches are invalid requests, not client
        // errors about the descriptor's contents.
        if let Value::Object(o) = descriptor {
            for key in ["fileName", "path", "contentHash", "parseOptionsKey", "nodeId"] {
                if !matches!(o.get(key), None | Some(Value::Null) | Some(Value::String(_))) {
                    return Err(ApiError::invalid_request(format!("cannot unmarshal into Go string within \"/file/{key}\"")));
                }
            }
            if !matches!(o.get("scriptKind"), None | Some(Value::Null) | Some(Value::Number(_))) {
                return Err(ApiError::invalid_request("cannot unmarshal into Go core.ScriptKind within \"/file/scriptKind\""));
            }
        } else if !matches!(descriptor, Value::Null) {
            return Err(ApiError::invalid_request("cannot unmarshal into Go api.SourceFileDescriptor within \"/file\""));
        }
        let key = parse_cache_key(descriptor).map_err(|e| ApiError::client(format!("invalid source file descriptor: {e}")))?;
        let lease = self.snapshot_host.acquire_existing_source_file(key).ok_or_else(|| ApiError::client("source file is not available"))?;
        if source_file_descriptor(lease.source_file()) != descriptor_normalized(descriptor) {
            lease.release();
            return Err(ApiError::client("source file descriptor no longer identifies the cached source file"));
        }
        Ok(lease)
    }

    fn create_source_file_lease(&self, file_name: &str, text: &str, options: &Value) -> ApiResult<SourceFileLease> {
        let sk = match Params(options).get("scriptKind") {
            Value::Null => ScriptKind::Unknown,
            Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 => script_kind_from(*n as u64).unwrap_or(ScriptKind::Unknown),
            _ => return Err(ApiError::invalid_request("scriptKind must be a number")),
        };
        let raw_kind = match Params(options).get("scriptKind") {
            Value::Number(n) => *n as i64,
            _ => 0,
        };
        let sk = if raw_kind == 0 { tsrs_core::ensure_script_kind_from_file_name(file_name) } else { sk };
        if !is_valid_create_script_kind(sk) || (raw_kind != 0 && script_kind_from(raw_kind as u64).is_none()) {
            return Err(ApiError::client(format!("invalid scriptKind {}", if raw_kind != 0 { raw_kind } else { sk as i64 })));
        }
        let file_name = tspath::get_normalized_absolute_path(file_name, self.current_directory());
        let options = SourceFileParseOptions { path: self.to_path(&file_name), file_name, ..Default::default() };
        Ok(self.snapshot_host.acquire_source_file(options, text, sk))
    }

    /// Go `encodeLeasedSourceFile`.
    fn encode_leased(&self, lease: SourceFileLease) -> ApiResult<Response> {
        let mut data = match self.encode_source_file(lease.source_file()) {
            Ok(d) => d,
            Err(e) => {
                lease.release();
                return Err(e);
            }
        };
        let id = self.register_lease(lease);
        codec::set_source_file_lease(&mut data, id);
        self.binary_or_base64(data)
    }

    pub(crate) fn handle_create_source_file(&self, p: Params) -> ApiResult<Response> {
        let lease = self.create_source_file_lease(p.str("fileName")?, p.str("sourceText")?, p.get("options"))?;
        self.encode_leased(lease)
    }

    pub(crate) fn handle_create_source_file_from_file(&self, p: Params) -> ApiResult<Response> {
        let file_name = tspath::get_normalized_absolute_path(p.str("fileName")?, self.current_directory());
        let text = self.base_fs().read_file(&file_name).ok_or_else(|| ApiError::client(format!("could not read file {file_name:?}")))?;
        let lease = self.create_source_file_lease(&file_name, &text, p.get("options"))?;
        self.encode_leased(lease)
    }

    pub(crate) fn handle_retain_source_file(&self, p: Params) -> ApiResult<Response> {
        let lease = self.acquire_cached_source_file(p.get("file"))?;
        let id = self.register_lease(lease);
        json_response(&Obj::new().set("lease", Value::Number(id as f64)).build())
    }

    pub(crate) fn handle_get_cached_source_file(&self, p: Params) -> ApiResult<Response> {
        let lease = self.acquire_cached_source_file(p.get("file"))?;
        let r = self.encode_source_file_response(Some(lease.source_file()));
        lease.release();
        r
    }

    pub(crate) fn handle_release_source_file(&self, p: Params) -> ApiResult<Response> {
        let id = p.u64("lease")?;
        if id == 0 {
            return Err(ApiError::client("empty source file lease"));
        }
        let lease = self.source_files.leases.lock().unwrap().remove(&id).ok_or_else(|| ApiError::client(format!("source file lease {id} not found")))?;
        lease.release();
        json_response(&Value::Bool(true))
    }

    pub(crate) fn handle_get_source_file(&self, p: Params) -> ApiResult<Response> {
        let sd = self.snapshot_data(p.u64("snapshot")?)?;
        let program = sd.get_program(&tsrs_project::ID(p.str("project")?.to_string()))?;
        let file = p.document("file")?;
        let r = self.encode_source_file_response(program.get_source_file(&file.to_file_name()));
        drop(sd);
        r
    }

    pub(crate) fn handle_get_config_source_file(&self, p: Params) -> ApiResult<Response> {
        let sd = self.snapshot_data(p.u64("snapshot")?)?;
        let program = sd.get_program(&tsrs_project::ID(p.str("project")?.to_string()))?;
        let file = p.document("file")?;
        let command_line = program.command_line();
        let Some(config) = command_line.config_file else { return self.encode_source_file_response(None) };
        let cs = self.use_case_sensitive_file_names();
        let cwd = program.get_current_directory();
        let requested = tspath::to_path(&file.to_file_name(), cwd, cs);
        if *config.source_file.path() == requested {
            return self.encode_source_file_response(Some(config.source_file));
        }
        for name in command_line.extended_source_files() {
            if tspath::to_path(&name, cwd, cs) != requested {
                continue;
            }
            let Some(text) = sd.snapshot.read_file(&name) else { return self.encode_source_file_response(None) };
            // Go parses a fresh tsconfig source file per call; here it lives in a scratch region freed after
            // encoding (its cached node index table is evicted with the region).
            let region = tsrs_core::arena::Region::new(64 << 10);
            let response = {
                let _scope = region.enter();
                let parsed = tsrs_tsoptions::new_tsconfig_source_file_from_file_path(&name, requested, &text);
                self.encode_source_file_response(Some(parsed.source_file))
            };
            drop(region);
            return response;
        }
        self.encode_source_file_response(None)
    }
}

/// Descriptors compare structurally with Go's struct equality; normalize field order/shape for comparison.
fn descriptor_normalized(v: &Value) -> Value {
    let p = Params(v);
    Obj::new()
        .set("fileName", p.get("fileName").clone())
        .set("path", p.get("path").clone())
        .set("contentHash", p.get("contentHash").clone())
        .set("parseOptionsKey", p.get("parseOptionsKey").clone())
        .set("scriptKind", p.get("scriptKind").clone())
        .set("nodeId", p.get("nodeId").clone())
        .build()
}
