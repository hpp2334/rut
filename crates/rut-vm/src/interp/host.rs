//! The embedder's host-fn binding table — built
//! BEFORE the `Vm` exists. An embedder is a host: its bindings depend on
//! nothing but itself (the closures capture their own state; the `&mut Vm`
//! in the body's type is a call-time argument), so registration happens up
//! front and [`Vm::new`](super::Vm::new) joins the table against the
//! program's host thunks eagerly — a declared-but-unbound fn is a
//! construction error, never a mid-run trap.
//!
//! [`HostRegistry::verify_against`] is the contract check, also
//! pre-VM: the session's declared `.d.rut` surface vs the registry,
//! panicking on the three mismatch classes before any rut code runs.

use std::any::Any;
use std::collections::{BTreeMap, HashMap};

use rut_core::types::TypeId;

use super::boundary::HostHandler;
use super::Vm;
use crate::heap::Slot;

/// A host fn is one dispatch-table entry — the same unit every other
/// table in the VM trades in (threaded op handlers, vtables): a code
/// pointer, a state word, and the declared return for the refcount law.
pub type Ctx = *const ();
pub type HostCode = fn(&mut Vm, &[Slot], Ctx) -> Slot;

/// A binding's signature — a fixed array, no heap: max host arity 8 is
/// the boundary's own law (tuples cap at arity 8). DERIVED from the
/// callable's Rust shape (`HostHandler::SIG`), never written.
#[derive(Clone, Copy)]
pub(crate) struct HostSig {
    pub tys: [TypeId; 8],
    pub ret: TypeId,
    pub n: u8,
}

impl HostSig {
    pub const fn new(tys: &[TypeId], ret: TypeId) -> HostSig {
        let mut a = [0; 8];
        let mut i = 0;
        while i < tys.len() && i < 8 {
            a[i] = tys[i];
            i += 1;
        }
        HostSig { tys: a, ret, n: tys.len() as u8 }
    }

    pub fn slice(&self) -> &[TypeId] {
        &self.tys[..self.n as usize]
    }
}

/// What [`HostRegistry::register`] produced: the table entry plus the
/// owner of its state word. `keep` moves into the Vm at the join and
/// pins the boxed body for the machine's lifetime (single thread,
/// the same trust the threaded loop's raw pointers run on).
pub(crate) struct HostBinding {
    pub code: HostCode,
    pub ctx: Ctx,
    pub sigs: HostSig,
    pub keep: Option<Box<dyn Any>>,
}

/// The dispatch-table row, dense by func idx after the join. Copy: the
/// hot path copies one entry out and dispatches.
#[derive(Clone, Copy)]
pub(crate) struct HostSlot {
    pub code: HostCode,
    pub ctx: Ctx,
    pub ret: TypeId,
    /// the join's verdict on `ret` — the `Vm::is_ref` predicate frozen
    /// once per binding (crossing-fastpath phase 2): the write-back
    /// refcount law in `call_host` reads this bit instead of doing a
    /// per-call `type_repr` lookup. The bool packs into the struct's
    /// existing 4-byte padding hole after `ret` — the entry stays
    /// 24 bytes (code + ctx + id), still `Copy`.
    pub ret_is_ref: bool,
}

impl HostSlot {
    /// filler for non-host funcs — never dispatched (the `host_id`
    /// discriminant in the `FuncCode` routes, as in `hotpath(2)`)
    fn never(_vm: &mut Vm, _slots: &[Slot], _ctx: Ctx) -> Slot {
        debug_assert!(false, "non-host funcs never dispatch through host_slots");
        Slot::int(0)
    }
    pub const NEVER: HostSlot = HostSlot {
        code: Self::never,
        ctx: std::ptr::null(),
        ret: 0,
        ret_is_ref: false,
    };
}

/// A host pkg's expected binding table: `<scope>::<name>` →
/// `(params, ret)` — the `.d.rut` declarations a mounting session holds,
/// checked against a registry by [`HostRegistry::verify_against`].
pub type ExpectedHostFns = BTreeMap<String, (Vec<TypeId>, TypeId)>;

/// The embedder's binding table, handed to `Vm::new` (fourth argument).
pub struct HostRegistry {
    fns: HashMap<String, HostBinding>,
    /// the scopes an [`HostPkg`] installer already claimed — one
    /// installer per pkg; a second `install_host_pkg` for the same
    /// scope is a wiring bug and panics
    installed_scopes: std::collections::HashSet<String>,
}

/// One host pkg's binding set. Rows register under UNPREFIXED names;
/// the scope comes solely from [`HostPkg::new`] — a pkg cannot spell
/// another pkg's rows (the anti-cross-injection law). The callable's
/// Rust shape IS the `.d.rut` row, as [`HostRegistry::register`].
pub struct HostPkg {
    scope: String,
    /// row name ("map_new", "http_send__start") → binding
    rows: BTreeMap<String, HostBinding>,
}

impl HostPkg {
    /// Start a builder for the pkg mounted under `scope` — the
    /// registration scope (the package name; there is no override).
    pub fn new(scope: &str) -> HostPkg {
        HostPkg { scope: scope.to_string(), rows: BTreeMap::new() }
    }

    /// [`register!`](crate::register)'s law, scope-relative: the name is
    /// the bare row name and the full registration name is formed once,
    /// at the install. Panics on a duplicate row name within the pkg —
    /// the flat registry's silent last-wins becomes a loud builder bug.
    pub fn register<F, A, R, K>(&mut self, name: &str, f: F)
    where
        F: HostHandler<A, R, K> + 'static,
    {
        if self.rows.contains_key(name) {
            panic!(
                "host pkg `{}` registers `{name}` twice — one row per name",
                self.scope
            );
        }
        let boxed = Box::new(f);
        // the heap pointee is stable for the box's life; the Box handle
        // itself moves into `keep` (the same shape `HostRegistry::register`)
        let ctx = &*boxed as *const F as Ctx;
        self.rows.insert(
            name.to_string(),
            HostBinding { code: F::entry, ctx, sigs: F::SIG, keep: Some(boxed) },
        );
    }

    /// the chain terminator
    pub fn build(self) -> HostPkg {
        self
    }
}

impl HostRegistry {
    pub fn new() -> HostRegistry {
        HostRegistry {
            fns: HashMap::new(),
            installed_scopes: std::collections::HashSet::new(),
        }
    }
}

impl Default for HostRegistry {
    fn default() -> HostRegistry {
        HostRegistry::new()
    }
}

impl HostRegistry {
    /// The magic: the callable's Rust shape IS the `.d.rut` row —
    /// `hosts.register("calc::abs", |_vm, x: f64| x.abs())`. The
    /// signature is DERIVED (`F::SIG`, a fixed array, no heap); the
    /// adapter `F::entry` IS the table entry. Infallible `-> R` and
    /// fallible `-> Result<R, Trap>` bodies both fit — `K` is solved by
    /// whichever impl the body's return type matches, and never written.
    /// A param type with no `Arg` impl is a compile error here. For
    /// two-plus params the compiler needs the marker spelled — use the
    /// [`register!`](crate::register) sugar and the closure stays plain.
    pub fn register<F, A, R, K>(&mut self, name: &str, f: F)
    where
        F: HostHandler<A, R, K> + 'static,
    {
        let boxed = Box::new(f);
        // the heap pointee is stable for the box's life; the Box handle
        // itself moves into `keep`
        let ctx = &*boxed as *const F as Ctx;
        self.fns.insert(
            name.to_string(),
            HostBinding { code: F::entry, ctx, sigs: F::SIG, keep: Some(boxed) },
        );
    }

    /// The `.d.rut` ↔ host-impl contract check, PANICKING
    /// before the Vm boots on three mismatch classes:
    ///
    /// - declared but unbound — a rut call would trap mid-run;
    /// - bound but undeclared — the pkg's surface lies about what exists;
    /// - signature drift — the crossing values would be misinterpreted.
    ///
    /// `expected` is the mounting session's table
    /// (`Session::expected_host_fns`). An embedder wiring bug is a
    /// panic, never a rut diagnostic.
    pub fn verify_against(&self, expected: &ExpectedHostFns) {
        for (name, (params, ret)) in expected {
            match self.fns.get(name) {
                None => panic!(
                    "host fn `{name}` is declared by a mounted package but never bound — register the body before Vm::new"
                ),
                Some(b) => {
                    if b.sigs.slice() != params.as_slice() || b.sigs.ret != *ret {
                        panic!(
                            "host fn `{name}` signature drift: the pkg declares {}, the binding is {}",
                            host_sig_str(params, *ret),
                            host_sig_str(b.sigs.slice(), b.sigs.ret)
                        );
                    }
                }
            }
        }
        for name in self.fns.keys() {
            if !expected.contains_key(name) {
                panic!(
                    "host fn `{name}` is bound but declared by no mounted package — the surface is missing"
                );
            }
        }
    }

    /// Install one built host pkg. The contract is ASYMMETRIC, by
    /// design:
    ///
    /// - scope IS mounted: this call must bind every DECLARED row
    ///   (declared-but-unbound ⇒ panic, naming the pkg) and every bound
    ///   row the decl names must match its signature (drift ⇒ panic —
    ///   this is the ONLY drift gate on the boot path; `Vm::new`'s
    ///   `take_binding` compares nothing). Rows the decl does not name
    ///   ride as inert extras.
    /// - scope NOT mounted: the whole pkg merges inert (the
    ///   blanket-install lane — the CLI installs all of them; a program
    ///   that mounts a subset carries the rest as dead bindings).
    ///
    /// Rows live under THIS pkg's scope alone; one installer per scope
    /// (a second install for the same scope panics — a wiring bug).
    pub fn install_host_pkg(&mut self, ctx: &HostPkgContext, pkg: HostPkg) {
        if self.installed_scopes.contains(&pkg.scope) {
            panic!(
                "host pkg `{}` is already installed — one installer per pkg",
                pkg.scope
            );
        }
        if let Some(declared) = ctx.rows_of(&pkg.scope) {
            for name in declared.keys() {
                if !pkg.rows.contains_key(name) {
                    panic!(
                        "host fn `{}::{name}` is declared by the mounted pkg but never bound — installer `{}` must register it",
                        pkg.scope, pkg.scope
                    );
                }
            }
            for (name, b) in &pkg.rows {
                if let Some((params, ret)) = declared.get(name) {
                    if b.sigs.slice() != params.as_slice() || b.sigs.ret != *ret {
                        panic!(
                            "host fn `{}::{name}` signature drift: the pkg declares {}, the binding is {}",
                            pkg.scope,
                            host_sig_str(params, *ret),
                            host_sig_str(b.sigs.slice(), b.sigs.ret)
                        );
                    }
                } // undeclared rows ride — inert extras
            }
        }
        for (name, b) in pkg.rows {
            // the full registration name is formed ONCE, at the merge
            self.fns.insert(format!("{}::{name}", pkg.scope), b);
        }
        self.installed_scopes.insert(pkg.scope);
    }

    /// Take one binding out (the `Vm::new` join consumes the table; the
    /// leftover entries are the embedder's business — `verify_against`
    /// is the check that they were all declared).
    pub(crate) fn take_binding(&mut self, name: &str) -> Option<HostBinding> {
        // take the keep box (its ctx word must stay alive — the FIRST
        // claim moves it into the Vm) but leave the entry: one binding
        // may legitimately back several FuncCodes — the decl row and
        // the compiler-minted thunk of the sleep yield are two bodies
        // of the same host fn
        let b = self.fns.get_mut(name)?;
        let keep = b.keep.take();
        Some(HostBinding { code: b.code, ctx: b.ctx, sigs: b.sigs, keep })
    }
}

/// [`register!`](crate::register)'s sugar for the [`HostPkg`] builder —
/// the installer lane's default spelling. The name is the BARE row
/// name; the scope comes solely from [`HostPkg::new`].
///
/// ```ignore
/// let mut pkg = HostPkg::new("calc");
/// rut_vm::pkg_fn!(pkg, "abs", (f64,) -> f64, |vm, x| x.abs());
/// hosts.install_host_pkg(&ctx, pkg.build());
/// ```
#[macro_export]
macro_rules! pkg_fn {
    ($pkg:expr, $name:literal, ($($t:ty),* $(,)?) -> $ret:ty, $closure:expr $(,)?) => {
        $pkg.register::<_, ($($t,)*), $ret, _>($name, $closure)
    };
}

/// [`register_async!`](crate::register_async)'s sugar for the
/// [`HostPkg`] builder: one row spelling → the five-row family
/// (`<name>` trap, `__start`, `__yield`, `__take`, `__cancel`), all
/// registered under bare row names — the scope prefixes at the merge.
#[macro_export]
macro_rules! pkg_async_fn {
    ($pkg:expr, $name:literal, ($($t:ty),* $(,)?) -> $ret:ty, $start:expr $(,)?) => {
        $crate::pkg_async_fn!($pkg, $name, ($($t,)*) -> $ret, $start,
            move |_c: $crate::Completer<$ret>| {})
    };
    ($pkg:expr, $name:literal, ($($t:ty),* $(,)?) -> $ret:ty, $start:expr, $abort:expr $(,)?) => {
        $crate::__register_async_rows!($pkg, $name, ($($t,)*), $ret, $start, $abort)
    };
}

/// Name a boot-table type id for a panic message (ids are stable).
fn host_ty_name(t: TypeId) -> String {
    use rut_core::types::*;
    match t {
        TY_NIL => "nil",
        TY_BOOL => "bool",
        TY_STR => "str",
        TY_BYTES => "bytes",
        TY_F32 => "f32",
        TY_F64 => "f64",
        TY_I8 => "i8",
        TY_I16 => "i16",
        TY_I32 => "i32",
        TY_I64 => "i64",
        TY_U8 => "u8",
        TY_U16 => "u16",
        TY_U32 => "u32",
        TY_U64 => "u64",
        TY_OPAQUE => "opaque",
        // the answer optionals (the legal-host-returns phase): the
        // boot rows carry their elem's shell name, the
        // `?` shape is named from the CONST
        rut_core::types::TY_OPT_STR => "?str",
        rut_core::types::TY_OPT_BYTES => "?bytes",
        rut_core::types::TY_OPT_OPAQUE => "?opaque",
        _ => return format!("#{t:?}"),
    }
    .to_string()
}

/// Spell a `(params) -> ret` signature for a panic message.
fn host_sig_str(params: &[TypeId], ret: TypeId) -> String {
    format!(
        "({}) -> {}",
        params.iter().map(|&t| host_ty_name(t)).collect::<Vec<_>>().join(", "),
        host_ty_name(ret)
    )
}

/// The mounted host pkgs' declared rows, partitioned by registration
/// scope — the session's answer to "what does each host pkg expect".
/// Built ONCE by the driver (`Session::host_pkg_context`), shared by
/// every install call. Opaque on purpose: `rows_of`/`is_mounted`/
/// `scopes`/`flatten` are the API.
///
/// Why a context object at all: `Session` lives in rut-driver while
/// the installer lane is rut-vm (rut-std sits on rut-vm only) — this
/// is the minimal vm-side value the driver distills its mount
/// knowledge into. Not the compiled `Program` (the registry is built
/// before/independently of any program; async family thunks are minted
/// at consumer call sites, so the program under-reports them). Not the
/// flat [`ExpectedHostFns`] (can't distinguish mounted-with-zero-rows
/// from not-mounted; slicing would need prefix matching). Snapshot
/// semantics: built once per boot lane; rebuild if mounts change after.
#[derive(Clone, Debug, Default)]
pub struct HostPkgContext {
    /// scope ("ink_host", "nmap_host", "calc") → row name → sig;
    /// async rows pre-expanded into their family
    scopes: BTreeMap<String, BTreeMap<String, (Vec<TypeId>, TypeId)>>,
}

impl HostPkgContext {
    /// Declare one row of the pkg mounted under `scope` — the
    /// driver's build-side API (the read side is `rows_of` et al.).
    /// Async rows are declared pre-expanded by the caller.
    pub fn declare(&mut self, scope: &str, name: &str, params: Vec<TypeId>, ret: TypeId) {
        self.scopes
            .entry(scope.to_string())
            .or_default()
            .insert(name.to_string(), (params, ret));
    }

    /// The declared rows of the pkg mounted under `scope` — `None`
    /// when the scope is not mounted (an install over it merges inert).
    pub fn rows_of(&self, scope: &str) -> Option<&BTreeMap<String, (Vec<TypeId>, TypeId)>> {
        self.scopes.get(scope)
    }

    /// Is the scope mounted at all? (the probe's per-iteration gate —
    /// optional, never correctness)
    pub fn is_mounted(&self, scope: &str) -> bool {
        self.scopes.contains_key(scope)
    }

    /// The mounted scopes, in name order.
    pub fn scopes(&self) -> impl Iterator<Item = &str> {
        self.scopes.keys().map(String::as_str)
    }

    /// The flat full-name table — the raw lane's `verify_against`
    /// input.
    pub fn flatten(&self) -> ExpectedHostFns {
        let mut out = BTreeMap::new();
        for (scope, rows) in &self.scopes {
            for (name, sig) in rows {
                out.insert(format!("{scope}::{name}"), sig.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod host_pkg_tests {
    use super::*;
    use crate::Trap;

    /// The declared shape of a pkg world: one scope with two rows
    /// (every row the pkg declares MUST be bound by its installer).
    fn two_pkg_ctx() -> HostPkgContext {
        use rut_core::types::TY_I64;
        let mut ctx = HostPkgContext::default();
        ctx.declare("calc", "abs", vec![TY_I64], TY_I64);
        ctx.declare("calc", "neg", vec![TY_I64], TY_I64);
        ctx
    }

    fn calc_pkg() -> HostPkg {
        let mut pkg = HostPkg::new("calc");
        pkg.register::<_, (i64,), i64, _>("abs", |_vm: &mut Vm, x: i64| x.abs());
        pkg.register::<_, (i64,), i64, _>("neg", |_vm: &mut Vm, x: i64| -x);
        pkg.build()
    }

    #[test]
    fn rows_register_under_the_pkg_scope_alone() {
        // auto-prefix: the builder's bare names merge as `<scope>::<name>`
        let ctx = two_pkg_ctx();
        let mut hosts = HostRegistry::new();
        hosts.install_host_pkg(&ctx, calc_pkg());
        // the full names resolve through take_binding's public twin:
        // verify_against passes only when the prefixed rows exist
        hosts.verify_against(&ctx.flatten());
    }

    #[test]
    #[should_panic(expected = "registers `abs` twice — one row per name")]
    fn a_duplicate_row_in_one_pkg_panics() {
        let mut pkg = HostPkg::new("calc");
        pkg.register::<_, (i64,), i64, _>("abs", |_vm: &mut Vm, x: i64| x);
        pkg.register::<_, (i64,), i64, _>("abs", |_vm: &mut Vm, x: i64| x + 1);
    }

    #[test]
    #[should_panic(expected = "host pkg `calc` is already installed — one installer per pkg")]
    fn a_second_install_of_a_scope_panics() {
        let ctx = two_pkg_ctx();
        let mut hosts = HostRegistry::new();
        hosts.install_host_pkg(&ctx, calc_pkg());
        hosts.install_host_pkg(&ctx, calc_pkg());
    }

    #[test]
    #[should_panic(expected = "host fn `calc::neg` is declared by the mounted pkg but never bound — installer `calc` must register it")]
    fn declared_but_unbound_panics_scoped() {
        let ctx = two_pkg_ctx();
        let mut pkg = HostPkg::new("calc");
        // `neg` left out — the mounted pkg's row never bound
        pkg.register::<_, (i64,), i64, _>("abs", |_vm: &mut Vm, x: i64| x.abs());
        let mut hosts = HostRegistry::new();
        hosts.install_host_pkg(&ctx, pkg.build());
    }

    #[test]
    #[should_panic(expected = "host fn `calc::abs` signature drift")]
    fn drift_panics_scoped() {
        use rut_core::types::{TY_I64, TY_STR};
        let mut ctx = HostPkgContext::default();
        ctx.declare("calc", "abs", vec![TY_STR], TY_I64); // the decl lies
        let mut hosts = HostRegistry::new();
        hosts.install_host_pkg(&ctx, calc_pkg());
    }

    #[test]
    fn an_unmounted_scope_merges_inert() {
        // the blanket-install lane: no panic, dead bindings ride
        let ctx = two_pkg_ctx();
        let mut hosts = HostRegistry::new();
        hosts.install_host_pkg(&ctx, calc_pkg()); // `calc` not in ctx
        // and the ctx's OWN scopes still check against what was installed
        let mut hosts2 = HostRegistry::new();
        hosts2.install_host_pkg(&ctx, calc_pkg());
        hosts2.verify_against(&ctx.flatten());
    }

    #[test]
    fn an_inert_extra_row_rides() {
        // rows the decl does not name are inert extras, never drift
        use rut_core::types::TY_I64;
        let mut ctx = HostPkgContext::default();
        ctx.declare("calc", "abs", vec![TY_I64], TY_I64);
        let mut pkg = HostPkg::new("calc");
        pkg.register::<_, (i64,), i64, _>("abs", |_vm: &mut Vm, x: i64| x.abs());
        pkg.register::<_, (i64,), i64, _>("undeclared_row", |_vm: &mut Vm, x: i64| x + 1);
        let mut hosts = HostRegistry::new();
        hosts.install_host_pkg(&ctx, pkg.build());
    }

    #[test]
    fn the_async_family_matches_the_context_expansion() {
        // `pkg_async_fn!` emits the five-row family; the ctx declares
        // the same expansion, so the install checks it row for row
        use crate::Completer;
        use rut_core::types::{TY_I32, TY_I64, TY_NIL, TY_OPAQUE};
        let mut ctx = HostPkgContext::default();
        ctx.declare("nmap_host", "probe", vec![TY_I64], TY_I64);
        ctx.declare("nmap_host", "probe__start", vec![TY_I64], TY_OPAQUE);
        ctx.declare("nmap_host", "probe__yield", vec![TY_OPAQUE, TY_OPAQUE], TY_I32);
        ctx.declare("nmap_host", "probe__take", vec![TY_OPAQUE], TY_I64);
        ctx.declare("nmap_host", "probe__cancel", vec![TY_OPAQUE], TY_NIL);

        let mut pkg = HostPkg::new("nmap_host");
        crate::pkg_async_fn!(pkg, "probe", (i64,) -> i64, move |x: i64| {
            let c = Completer::<i64>::new();
            c.complete(x);
            c
        });
        let mut hosts = HostRegistry::new();
        hosts.install_host_pkg(&ctx, pkg.build());
        hosts.verify_against(&ctx.flatten());

        // the installed names are exactly the family, scoped
        let flat = ctx.flatten();
        for suffix in ["", "__start", "__yield", "__take", "__cancel"] {
            assert!(flat.contains_key(&format!("nmap_host::probe{suffix}")));
        }
    }

    #[test]
    fn the_async_abort_form_forwards_too() {
        // the two-closure spelling (start + abort) rides the same emitter
        use crate::Completer;
        use rut_core::types::{TY_I32, TY_I64, TY_NIL, TY_OPAQUE};
        let mut ctx = HostPkgContext::default();
        ctx.declare("nmap_host", "probe", vec![TY_I64], TY_I64);
        ctx.declare("nmap_host", "probe__start", vec![TY_I64], TY_OPAQUE);
        ctx.declare("nmap_host", "probe__yield", vec![TY_OPAQUE, TY_OPAQUE], TY_I32);
        ctx.declare("nmap_host", "probe__take", vec![TY_OPAQUE], TY_I64);
        ctx.declare("nmap_host", "probe__cancel", vec![TY_OPAQUE], TY_NIL);

        let mut pkg = HostPkg::new("nmap_host");
        crate::pkg_async_fn!(pkg, "probe", (i64,) -> i64,
            move |x: i64| {
                let c = Completer::<i64>::new();
                c.complete(x);
                c
            },
            move |_c: Completer<i64>| {});
        let mut hosts = HostRegistry::new();
        hosts.install_host_pkg(&ctx, pkg.build());
        hosts.verify_against(&ctx.flatten());
    }
}
