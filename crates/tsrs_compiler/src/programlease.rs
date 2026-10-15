use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_core::context::Context;
use tsrs_core::P;

use crate::{EmitOptions, EmitResult, Program};

/// Keeps a program and its regions alive while exposing only borrowed access to it.
///
/// Project programs retain their existing region owner; batch programs can use [`Self::from_static`].
/// This protects program and checker acquisition. Arena results (`P<T>`) still require their existing
/// region and request leases and must not be used after their owner is released.
///
/// Both pooled and independent checkers borrow the lease, not the temporary context:
/// ```
/// use tsrs_compiler::{Context, ProgramLease};
/// fn check(lease: &ProgramLease) {
///     let checker = lease.get_type_checker(&Context::background());
///     drop(checker);
///     let checker = lease.new_checker();
///     drop(checker);
/// }
/// ```
///
/// A program reference cannot escape its lease:
/// ```compile_fail
/// use tsrs_compiler::{Program, ProgramLease};
/// fn escape(lease: ProgramLease) -> &'static Program {
///     lease.program()
/// }
/// ```
/// Nor can an acquired checker:
/// ```compile_fail
/// use tsrs_compiler::{CheckerHandle, Context, ProgramLease};
/// fn escape(lease: ProgramLease) -> CheckerHandle<'static> {
///     lease.get_type_checker(&Context::background())
/// }
/// ```
/// The owner cannot be dropped while a checker is still held:
/// ```compile_fail
/// use tsrs_compiler::{Context, ProgramLease};
/// fn release_while_checking(lease: ProgramLease) {
///     let checker = lease.get_type_checker(&Context::background());
///     drop(lease);
///     drop(checker);
/// }
/// ```
/// Independent checkers have the same owner requirement:
/// ```compile_fail
/// use tsrs_compiler::ProgramLease;
/// fn release_while_checking(lease: ProgramLease) {
///     let checker = lease.new_checker();
///     drop(lease);
///     drop(checker);
/// }
/// ```
#[derive(Clone)]
pub struct ProgramLease {
    program: &'static Program,
    _owner: Option<Arc<dyn Send + Sync>>,
}

impl ProgramLease {
    /// Retain a program with a separately managed region owner.
    ///
    /// # Safety
    /// `owner` must keep this exact program, its checker pool and all its regions alive until the last
    /// owner reference is dropped. No other path may free them while an owner reference exists.
    pub unsafe fn from_owner(program: &'static Program, owner: Arc<dyn Send + Sync>) -> Self {
        Self { program, _owner: Some(owner) }
    }

    /// Wrap a program that remains alive for the process lifetime. Reclaimable programs must use
    /// [`Self::from_owner`] instead; their internal legacy references are not truly static.
    pub fn from_static(program: &'static Program) -> Self {
        Self { program, _owner: None }
    }

    pub fn program(&self) -> &Program {
        self.program
    }

    /// Create an independent checker that borrows this program lease (used by auto-import indexing).
    pub fn new_checker(&self) -> ProgramChecker<'_> {
        ProgramChecker { checker: tsrs_checker::new_checker(self.program), _program: PhantomData }
    }

    // These compiler entry points still require a legacy static receiver internally (including the
    // arena-backed emit host). Keep that reference inside this module; the lease covers the whole call.
    pub fn get_semantic_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.program.get_semantic_diagnostics(ctx, file)
    }

    pub fn get_suggestion_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.program.get_suggestion_diagnostics(ctx, file)
    }

    pub fn get_declaration_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.program.get_declaration_diagnostics(ctx, file)
    }

    pub fn get_global_diagnostics(&self, ctx: &Context) -> Vec<P<Diagnostic>> {
        self.program.get_global_diagnostics(ctx)
    }

    pub fn get_program_diagnostics(&self) -> Vec<P<Diagnostic>> {
        self.program.get_program_diagnostics()
    }

    pub fn get_include_processor_diagnostics(&self, file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        self.program.get_include_processor_diagnostics(file)
    }

    pub fn emit(&self, ctx: &Context, options: &EmitOptions) -> EmitResult {
        self.program.emit(ctx, options)
    }
}

/// An independent checker whose program owner must remain alive until it is dropped.
pub struct ProgramChecker<'program> {
    checker: Box<tsrs_checker::Checker>,
    _program: PhantomData<&'program ProgramLease>,
}

impl Deref for ProgramChecker<'_> {
    type Target = tsrs_checker::Checker;

    fn deref(&self) -> &Self::Target {
        &self.checker
    }
}

impl DerefMut for ProgramChecker<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.checker
    }
}

impl Deref for ProgramLease {
    type Target = Program;

    fn deref(&self) -> &Program {
        self.program()
    }
}
