//! The VM's construction door — `Vm::builder()` and the
//! [`IntoVmParts`] seam. `Vm::new` is crate-internal now; every
//! construction goes through the builder:
//!
//! ```ignore
//! let vm = Vm::builder()
//!     .compiled(compiled)   // anything IntoVmParts (the driver's Compiled)
//!     .limits(limits)       // unset ⇒ Limits::default() — uncapped
//!     .hooks(hooks)
//!     .hosts(registry)
//!     .build()?;
//! ```
//!
//! Two construction shapes, one door: `.program(..)` (+ the raw
//! `.limits`/`.hooks`/`.hosts`) for embedders holding a decoded
//! [`Program`], and `.compiled(..)` for anything that hands over its
//! parts as a value — the driver's `RutRun::new()..compile()` product
//! implements [`IntoVmParts`] (the seam trait is defined HERE, in
//! rut-vm, and implemented there — the orphan-legal direction). A
//! `.compiled(..)` part fills whichever of the four the source
//! provided; the explicit setters always win.

use std::rc::Rc;

use rut_core::binary::Program;

use super::{HostHooks, HostRegistry, Limits, Vm};

/// Why a [`VmBuilder::build`] refused: a boot failure (a declared host
/// fn never bound, a const that cannot materialize) or a shape failure
/// (no program was ever given). `.msg` is the text; boot-time traps
/// carry their message verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmError {
    pub msg: String,
}

impl VmError {
    pub fn new(msg: impl Into<String>) -> VmError {
        VmError { msg: msg.into() }
    }
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for VmError {}

/// The four construction parts, handed over as one value by anything
/// implementing [`IntoVmParts`].
pub struct VmParts {
    pub program: Rc<Program>,
    pub limits: Limits,
    pub hooks: HostHooks,
    pub hosts: HostRegistry,
}

/// The seam: hand the builder a `VmParts`. Defined in rut-vm,
/// implemented by the driver (for its `Compiled`) — the value flows
/// compile → boot without the driver ever naming `Vm`'s fields.
pub trait IntoVmParts {
    fn into_vm_parts(self) -> Result<VmParts, VmError>;
}

/// The fluent builder — `Vm::builder()`. Unset `limits` mean
/// `Limits::default()` (uncapped — the mechanism law), unset `hooks`/
/// `hosts` mean their defaults.
#[derive(Default)]
pub struct VmBuilder {
    parts: Option<Result<VmParts, VmError>>,
    program: Option<Rc<Program>>,
    limits: Option<Limits>,
    hooks: Option<HostHooks>,
    hosts: Option<HostRegistry>,
}

impl VmBuilder {
    /// Take the four parts from a value that implements
    /// [`IntoVmParts`] (the driver's `Compiled`). Explicit
    /// `.limits`/`.hooks`/`.hosts` calls override the taken values;
    /// the program cannot be overridden by `.program` after this (the
    /// compiled product IS the program's owner).
    pub fn compiled(mut self, parts: impl IntoVmParts) -> Self {
        self.parts = Some(parts.into_vm_parts());
        self
    }

    /// The program to boot — the raw shape's head.
    pub fn program(mut self, program: Rc<Program>) -> Self {
        self.program = Some(program);
        self
    }

    /// Fuel/heap/interrupt budgets. Unset ⇒ uncapped.
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// The host hooks.
    pub fn hooks(mut self, hooks: HostHooks) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// The host-fn binding table, joined against the program's host
    /// thunks at boot.
    pub fn hosts(mut self, hosts: HostRegistry) -> Self {
        self.hosts = Some(hosts);
        self
    }

    /// Boot. The join runs here: every host thunk the program declares
    /// must resolve against the registry, so a declared-but-unbound fn
    /// is a construction error, never a mid-run trap.
    pub fn build(self) -> Result<Vm, VmError> {
        let mut program = self.program;
        let mut limits = self.limits;
        let mut hooks = self.hooks;
        let mut hosts = self.hosts;
        if let Some(parts) = self.parts {
            let parts = parts?;
            if program.is_none() {
                program = Some(parts.program);
            }
            if limits.is_none() {
                limits = Some(parts.limits);
            }
            if hooks.is_none() {
                hooks = Some(parts.hooks);
            }
            if hosts.is_none() {
                hosts = Some(parts.hosts);
            }
        }
        let program = program
            .ok_or_else(|| VmError::new("no program — the builder needs .program(..) or .compiled(..)"))?;
        Vm::new(
            program,
            &limits.unwrap_or_default(),
            hooks.unwrap_or_default(),
            hosts.unwrap_or_default(),
        )
        .map_err(|t| VmError { msg: t.msg })
    }
}
