// Port of the registries in Go `snapshotData` / `projectRegistryData` (tsc/internal/api/session.go).
//
// Symbol ids come from `tsrs_ast::get_symbol_id` (a process-wide counter, like Go `ast.GetSymbolId`), so
// snapshot-owned symbols are registered snapshot-wide. Type and signature ids are per-checker sequential
// ids, so they are registered per project, and additionally pinned to the id of the API checker that
// produced them: if the project's persistent API checker is replaced (e.g. after cancellation), every
// handle produced by the old checker becomes a stale-handle client error instead of silently resolving
// against a different checker.
//
// Each project's type/signature maps retain their program owner. Resolved handles retain it separately,
// so releasing the snapshot or replacing the checker cannot invalidate an already resolved result.
// Symbols retain the snapshot family and temporary checker data together. After release every new
// lookup fails, while already resolved handles keep their owners independently.

use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;
use tsrs_ast::Symbol;
use tsrs_checker::{Signature, Type};
use tsrs_core::P;

use super::host::{CheckerError, CheckerResult};

mod results;
pub use results::{SignatureHandle, SymbolHandle, TypeHandle};
pub(super) use results::{ResultOwner, SymbolOwner};

struct ProjectRegistry {
    owner: ResultOwner,
    types: FxHashMap<u32, P<Type>>,
    signatures: FxHashMap<u64, P<Signature>>,
}

#[derive(Default)]
struct RegistryState {
    released: bool,
    symbols: FxHashMap<u64, P<Symbol>>,
    // Go `symbolCanonicalProjects`: first writer wins.
    symbol_projects: FxHashMap<u64, String>,
    projects: FxHashMap<String, ProjectRegistry>,
    symbol_owner: Option<Arc<SymbolOwner>>,
}

#[derive(Default)]
pub struct CheckerRegistry {
    state: Mutex<RegistryState>,
}

impl CheckerRegistry {
    #[cfg(test)]
    pub fn new() -> CheckerRegistry {
        CheckerRegistry::default()
    }

    /// Drops every registered pointer. Runs when the snapshot's `SnapshotData` is dropped.
    pub fn release(&self) {
        let mut st = self.lock();
        st.released = true;
        st.symbols = FxHashMap::default();
        st.symbol_projects = FxHashMap::default();
        let projects = std::mem::take(&mut st.projects);
        let symbols = st.symbol_owner.take();
        drop(st);
        // Last leases may destroy checker pools and regions; do that outside the registry lock.
        drop(projects);
        drop(symbols);
    }

    /// Retain the base snapshot or an auto-import snapshot before registering any of its symbols.
    pub(super) fn symbol_owner(&self, snapshot: &Arc<tsrs_project::Snapshot>) -> CheckerResult<Arc<SymbolOwner>> {
        let mut st = self.lock();
        Self::check_live(&st)?;
        let owner = st.symbol_owner.get_or_insert_with(|| Arc::new(SymbolOwner::default()));
        owner.retain_snapshot(snapshot);
        Ok(Arc::clone(owner))
    }

    #[cfg(test)]
    pub fn is_released(&self) -> bool {
        self.lock().released
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryState> {
        // A panic while holding this short-lived lock cannot leave the maps half-updated in a way that
        // matters (every operation is a single insert/lookup), so recover from poisoning.
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn check_live(st: &RegistryState) -> CheckerResult<()> {
        if st.released {
            return Err(CheckerError::client("snapshot has been released"));
        }
        Ok(())
    }

    /// Go `registerSymbol`. Returns the symbol id and its canonical project.
    ///
    /// # Safety
    /// The symbol's source snapshot must have been retained by `owner`. Checker-created symbols must
    /// come from a persistent API checker or a query using `owner.checker_data`.
    pub(super) unsafe fn register_symbol(&self, symbol: P<Symbol>, canonical_project: &str, owner: &Arc<SymbolOwner>) -> CheckerResult<(u64, String)> {
        assert!(!canonical_project.is_empty(), "registerSymbol requires a non-empty canonical project");
        let id = tsrs_ast::get_symbol_id(symbol).0;
        let mut st = self.lock();
        Self::check_live(&st)?;
        assert!(st.symbol_owner.as_ref().is_some_and(|current| Arc::ptr_eq(current, owner)), "symbol belongs to another registry");
        match st.symbols.entry(id) {
            Entry::Occupied(entry) => assert!(*entry.get() == symbol, "duplicate symbol"),
            Entry::Vacant(entry) => {
                entry.insert(symbol);
            }
        }
        let project = st.symbol_projects.entry(id).or_insert_with(|| canonical_project.to_string()).clone();
        Ok((id, project))
    }

    /// Go `resolveSymbolHandle`.
    pub fn resolve_symbol(&self, id: u64) -> CheckerResult<SymbolHandle> {
        if id == 0 {
            return Err(CheckerError::client("empty symbol handle"));
        }
        let st = self.lock();
        Self::check_live(&st)?;
        st.symbols.get(&id).map(|&value| SymbolHandle { value, owner: Arc::clone(st.symbol_owner.as_ref().unwrap()) })
            .ok_or_else(|| CheckerError::client(format!("symbol handle {id} not found in snapshot registry")))
    }

    fn project_for_checker<'a>(st: &'a mut RegistryState, project: &str, owner: &ResultOwner) -> &'a mut ProjectRegistry {
        let reg = st.projects.entry(project.to_string()).or_insert_with(|| ProjectRegistry {
            owner: owner.clone(), types: FxHashMap::default(), signatures: FxHashMap::default(),
        });
        if !reg.owner.matches(owner) {
            // The project's API checker changed: handles from the previous checker are stale.
            *reg = ProjectRegistry { owner: owner.clone(), types: FxHashMap::default(), signatures: FxHashMap::default() };
        }
        reg
    }

    /// Go `registerType`.
    ///
    /// # Safety
    /// `t` and its reachable graph must belong to the persistent API checker retained by `owner`.
    pub(super) unsafe fn register_type(&self, project: &str, owner: &ResultOwner, t: P<Type>) -> CheckerResult<u32> {
        assert!(!project.is_empty(), "registerType: empty project ID");
        let id = t.id().0;
        let mut st = self.lock();
        Self::check_live(&st)?;
        let reg = Self::project_for_checker(&mut st, project, owner);
        match reg.types.entry(id) {
            Entry::Occupied(entry) => assert!(*entry.get() == t, "duplicate type"),
            Entry::Vacant(entry) => {
                entry.insert(t);
            }
        }
        Ok(id)
    }

    /// Go `resolveTypeHandle`. With `checker_id = Some(id)` the handle must come from that checker.
    pub fn resolve_type(&self, project: &str, id: u32, checker_id: Option<u32>) -> CheckerResult<TypeHandle> {
        if id == 0 {
            return Err(CheckerError::client("empty type handle"));
        }
        if project.is_empty() {
            return Err(CheckerError::client(format!("empty project ID for type handle {id}")));
        }
        let st = self.lock();
        Self::check_live(&st)?;
        let Some(reg) = st.projects.get(project) else {
            return Err(CheckerError::client(format!("type handle {id} not found (no registry for project {project})")));
        };
        if checker_id.is_some_and(|c| c != reg.owner.checker_id) {
            return Err(CheckerError::client(format!("type handle {id} is stale (the project's API checker was replaced)")));
        }
        reg.types.get(&id).map(|&value| TypeHandle { value, owner: reg.owner.clone() })
            .ok_or_else(|| CheckerError::client(format!("type handle {id} not found in project registry")))
    }

    /// Go `registerSignature`.
    ///
    /// # Safety
    /// `sig` and its reachable graph must belong to the persistent API checker retained by `owner`.
    pub(super) unsafe fn register_signature(&self, project: &str, owner: &ResultOwner, sig: P<Signature>) -> CheckerResult<u64> {
        assert!(!project.is_empty(), "registerSignature: empty project ID");
        let id = sig.id().0 as u64;
        let mut st = self.lock();
        Self::check_live(&st)?;
        let reg = Self::project_for_checker(&mut st, project, owner);
        match reg.signatures.entry(id) {
            Entry::Occupied(entry) => assert!(*entry.get() == sig, "duplicate signature"),
            Entry::Vacant(entry) => {
                entry.insert(sig);
            }
        }
        Ok(id)
    }

    /// Go `resolveSignatureHandle`.
    pub fn resolve_signature(&self, project: &str, id: u64, checker_id: Option<u32>) -> CheckerResult<SignatureHandle> {
        if id == 0 {
            return Err(CheckerError::client("empty signature handle"));
        }
        if project.is_empty() {
            return Err(CheckerError::client(format!("empty project ID for signature handle {id}")));
        }
        let st = self.lock();
        Self::check_live(&st)?;
        let Some(reg) = st.projects.get(project) else {
            return Err(CheckerError::client(format!("signature handle {id} not found (no registry for project {project})")));
        };
        if checker_id.is_some_and(|c| c != reg.owner.checker_id) {
            return Err(CheckerError::client(format!("signature handle {id} is stale (the project's API checker was replaced)")));
        }
        reg.signatures.get(&id).map(|&value| SignatureHandle { value, owner: reg.owner.clone() })
            .ok_or_else(|| CheckerError::client(format!("signature handle {id} not found in project registry")))
    }

    /// Register a target reached from a resolved signature without switching the active generation.
    pub(super) fn register_signature_target(&self, project: &str, sig: &SignatureHandle) -> CheckerResult<u64> {
        let id = sig.id();
        let mut st = self.lock();
        Self::check_live(&st)?;
        let reg = st.projects.get_mut(project).filter(|reg| reg.owner.matches(&sig.owner))
            .ok_or_else(|| CheckerError::client(format!("signature handle {id} is stale (the project's API checker was replaced)")))?;
        match reg.signatures.entry(id) {
            Entry::Occupied(entry) => assert!(*entry.get() == sig.value, "duplicate signature"),
            Entry::Vacant(entry) => { entry.insert(sig.value); }
        }
        Ok(id)
    }

    /// Checker that produced the project's registered types/signatures (0 if none yet).
    #[cfg(test)]
    pub fn project_checker_id(&self, project: &str) -> u32 {
        self.lock().projects.get(project).map_or(0, |r| r.owner.checker_id)
    }
}
