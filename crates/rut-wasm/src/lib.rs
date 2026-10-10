//! rut-wasm — the demo page's compile/run surface, mirrored
//! by `demo/src/wasm/rut-api.d.ts`.
//!
//! Raw ABI over wasm32 linear memory (no wasm-bindgen glue):
//!   rut_alloc(len) -> ptr                      bump allocation
//!   rut_compile(src_ptr, src_len) -> result    JSON envelope, base64 binary
//!   rut_run(bin_ptr, bin_len, fuel, heap) -> result
//!   rut_resume(extra_fuel, heap) -> result     continue the parked frame
//!   rut_drop_frame() -> u32                    retire the parked frame
//! Every result is [u32 little-endian length][bytes] at the returned ptr.
//! Run envelopes carry `"parked":bool` — true when the guest trapped
//! OutOfFuel with a live frame that `rut_resume` can continue — and
//! `"err"` beside `"trap"` (err-channel phase 3): a `main` returning the
//! `(?T, err)` pair shape surfaces its non-empty err there, while panics
//! and budget traps stay on `trap` — the two channels never mix.

#![allow(static_mut_refs)]

use std::rc::Rc;

// the positional driver decode (`Ret for Value`) — the run envelope's
// entry-err reader; aliased so the host's own JSON never collides
use rut_driver::CompileOutput;
use rut_vm::Value as RutValue;

// ---- bump allocator over linear memory ----

const HEAP_SIZE: usize = 16 * 1024 * 1024;
#[no_mangle]
static mut RUT_HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
static mut HEAP_TOP: usize = 0;

#[no_mangle]
pub extern "C" fn rut_alloc(len: usize) -> *mut u8 {
    unsafe {
        let top = HEAP_TOP;
        if top + len + 8 > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        HEAP_TOP = top + len;
        RUT_HEAP.as_mut_ptr().add(top)
    }
}

// ---- JSON envelope writer (no serde on this path) ----

fn json_escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// stash the last result envelope and return its pointer
fn envelope(bytes: &[u8]) -> *mut u8 {
    unsafe {
        let top = HEAP_TOP;
        if top + bytes.len() + 8 > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        let len = (bytes.len() as u32).to_le_bytes();
        RUT_HEAP[top..top + 4].copy_from_slice(&len);
        RUT_HEAP[top + 4..top + 4 + bytes.len()].copy_from_slice(bytes);
        HEAP_TOP = top + 4 + bytes.len();
        RUT_HEAP.as_mut_ptr().add(top)
    }
}

unsafe fn read_str<'a>(ptr: *const u8, len: usize) -> &'a str {
    let slice = core::slice::from_raw_parts(ptr, len);
    core::str::from_utf8(slice).unwrap_or("")
}

// ---- compile ----

/// The playground's library offers (§0.9 of the host-pkgs plan): wasm
/// has no filesystem, so the toolchain libs this host ships are
/// EMBEDDED — `ink_host`'s and `nmap_host`'s host surfaces lowered from
/// their `.d.rut`s, `ink`, `pouch` and `nmapset` as in-memory source.
/// This host's choice, not the engine's: the driver knows none of these
/// names. (`nmap_host` + `nmapset` are the survey D6 amendment — the
/// demo's map lane; `rut_std::nmap::pkg()` binds the crossings in `rut_run`.)
fn compile_playground(src: &str) -> rut_driver::CompileOutput {
    // the string lane's own render: the tree UI's structured AST rides
    // every envelope, diags or not
    let (ast, parse_diags) = rut_parser::parse(src, rut_parser::Mode::Impl);
    let tree = rut_ast::dump::to_dump_tree(&ast);
    let ast_json = rut_ast::dump::render_json(&tree);
    if !parse_diags.is_empty() {
        return CompileOutput {
            diags: parse_diags,
            ast_json,
            ast_dump: String::new(),
            ir_dump: String::new(),
            binary: None,
        };
    }
    let named = |spec: &str, pkg: rut_driver::Pkg| -> rut_driver::Pkg {
        pkg.named(spec)
    };
    // the `.compile()` product's host registry is the run-side install
    // snapshot: `rut_run` takes it (the one registry per compile) and
    // boots with it — the ctx ceremony is gone, the registry travels
    let ink_host = named(
        "ink_host",
        rut_driver::lower_decl_module(
            include_str!("../../../rut/ink_host/ink_host.d.rut"),
            "ink_host.d.rut",
        )
        .expect("the ink_host surface is valid"),
    );
    // the nmap lane (survey D6): the host surface lowers exactly like
    // `ink_host` — its registration scope is the default (the pkg
    // spec `nmap_host`), the scope `rut_std::nmap::pkg()` installs under
    let nmap_host = named(
        "nmap_host",
        rut_driver::lower_decl_module(
            include_str!("../../../rut/nmap_host/nmap.d.rut"),
            "nmap.d.rut",
        )
        .expect("the nmap_host surface is valid"),
    );
    // `flow` offers AFTER its deps (its generic class + traits ride the
    // source). The browser keeps no filesystem, so the peer-gate's
    // directory walk can't run here: the manifest's peer-deps record
    // onto the pkg, and each integration group whose optional peer this
    // world holds rides its include_str! text — the same append the
    // gate pass performs from disk, with the gate flag set so the
    // close-of-world pass never double-appends (`Vec.from_flow`
    // resolves through the group rows; the book gate's lane proves
    // the law).
    let flow_manifest = rut_driver::parse_manifest(include_str!("../../../rut/flow/rut.jsonc"))
        .expect("the flow manifest is valid");
    let mut flow = rut_driver::Pkg::source("flow", include_str!("../../../rut/flow/mod.rut"));
    flow.entry = flow_manifest.entry.clone();
    for (peer, desc) in &flow_manifest.peer_deps {
        flow.peers.insert(peer.clone(), rut_driver::PeerDecl {
            optional: desc.get("optional").map(|v| v == "true").unwrap_or(false),
            lib: desc.get("lib").cloned(),
            path: desc.get("path").cloned().unwrap_or_default(),
        });
    }
    flow.peer_groups.push(include_str!("../../../rut/flow/group-pouch.rut").to_string());
    flow.peer_groups.push(include_str!("../../../rut/flow/group-nmapset.rut").to_string());
    flow.groups_mounted = true;
    // the async set: the engine rows lower from their decl,
    // the typed launcher surface offers as a linked source pkg —
    // `rut_std::async_host::pkg()` binds the crossings in `rut_run`
    let async_host = named(
        "async_host",
        rut_driver::lower_decl_module(
            include_str!("../../../rut/async_host/engine.d.rut"),
            "engine.d.rut",
        )
        .expect("the async_host surface is valid"),
    );
    // the book lane (the run buttons in docs/): `json` and `strbuild`
    // complete the CLI's loose-file host set — a book block that
    // `use json::` / `use strbuild::` gets the same packages `rut run`
    // offers, so the book's buttons answer exactly like the CLI. The
    // HTTP pair stays CLI-only by law: reqwest does not build on
    // wasm32-unknown-unknown (this crate's own dependency note), so a
    // `use http::` block fails to resolve here — loud, never silent.
    // strbuild's rows ride the `strbuild_host` decl surface (the
    // ink/ink_host pattern), lowered here exactly like `ink_host`
    // above; the bodies bind in `rut_run` (`rut_std::strbuild::pkg()`)
    let strbuild_host = named(
        "strbuild_host",
        rut_driver::lower_decl_module(
            include_str!("../../../rut/strbuild_host/strbuild_host.d.rut"),
            "strbuild_host.d.rut",
        )
        .expect("the strbuild_host surface is valid"),
    );
    // calc rides as a lowered surface + the manifest's namespace/consts
    // rows — the same shape a `tree_pkg("calc")` walk yields (the wasm
    // host has no filesystem, so the files ride `include_str!`; the
    // core prelude auto-offers in `.compile()`)
    let calc_manifest = rut_driver::parse_manifest(include_str!("../../../rut/calc/rut.jsonc"))
        .expect("the calc manifest is valid");
    let mut calc = rut_driver::lower_decl_module(
        include_str!("../../../rut/calc/calc.d.rut"),
        "calc.d.rut",
    )
    .expect("the calc surface is valid");
    calc.namespace = calc_manifest.namespace.clone();
    if let rut_driver::PkgBody::Host { consts, .. } = &mut calc.body {
        *consts = calc_manifest
            .consts
            .iter()
            .map(|(n, v)| (n.clone(), rut_core::types::TY_F64, v.to_bits()))
            .collect();
    }
    let calc = named("calc", calc);
    // the library world (everything but the program's own source): the
    // run-side install snapshot reads its declared rows
    let lib_pkgs = vec![
        ink_host,
        nmap_host,
        rut_driver::Pkg::source("ink", include_str!("../../../rut/ink/mod.rut")),
        rut_driver::Pkg::source("pouch", include_str!("../../../rut/pouch/mod.rut")),
        // `nmapset` links like `ink` (its generic classes' methods cross on
        // the surface's inherent rows; a consumer requests the
        // instantiations); its `use nmap_host::` resolves
        // against the offered surface above
        rut_driver::Pkg::source("nmapset", include_str!("../../../rut/nmapset/mod.rut")),
        flow,
        async_host,
        rut_driver::Pkg::source("futures", include_str!("../../../rut/futures/mod.rut")),
        strbuild_host,
        rut_driver::Pkg::source("strbuild", include_str!("../../../rut/strbuild/mod.rut")),
        rut_driver::Pkg::source("json", include_str!("../../../rut/json/mod.rut")),
        calc,
    ];
    // the rows snapshot `rut_run`'s installs answer to (owned; the
    // compile-time world dies here, the table travels)
    *CTX.lock().unwrap() = Some(rut_driver::host_pkg_ctx(&lib_pkgs));
    let compiled = rut_driver::RutRun::new()
        .pkgs(&rut_driver::Loaded { pkgs: lib_pkgs, root: String::new() })
        .pkg(rut_driver::Pkg::source("main", src))
        .entrypoint("main")
        .compile();
    match compiled {
        Ok(c) => {
            let ir_dump = c
                .graph
                .program
                .as_ref()
                .map(|p| rut_driver::ir_dump_of(&p.funcs, &p.interner))
                .unwrap_or_default();
            CompileOutput {
                diags: c.graph.diags,
                ast_json,
                ast_dump: String::new(),
                ir_dump,
                binary: c.graph.program.map(|p| rut_core::binary::encode(&p)),
            }
        }
        Err(e) => CompileOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            ast_json,
            ast_dump: String::new(),
            ir_dump: String::new(),
            binary: None,
        },
    }
}

#[no_mangle]
pub extern "C" fn rut_compile(src_ptr: *const u8, src_len: usize) -> *mut u8 {
    let src = unsafe { read_str(src_ptr, src_len) };
    let out = compile_playground(src);
    let mut json = String::from("{\"diags\":[");
    for (i, d) in out.diags.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"start\":{},\"end\":{},\"msg\":",
            d.span.lo, d.span.hi
        ));
        json_escape(&d.msg, &mut json);
        json.push('}');
    }
    json.push_str("],\"ast\":");
    json.push_str(&out.ast_json); // raw splice — it IS valid JSON
    json.push_str(",\"irDump\":");
    json_escape(&out.ir_dump, &mut json);
    match &out.binary {
        Some(b) => {
            json.push_str(",\"binary\":\"");
            json.push_str(&base64(b));
            json.push_str("\"}");
        }
        None => json.push('}'),
    }
    envelope(json.as_bytes())
}

// ---- run ----

static OUTPUT: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

/// The mount snapshot the compile lane built — the installs in
/// `rut_run` answer to it (the ctx is OWNED: the compile-time world is
/// long gone by the time the page presses Run, and every run installs
/// a fresh registry against it).
static CTX: std::sync::Mutex<Option<rut_vm::interp::HostPkgContext>> =
    std::sync::Mutex::new(None);

/// The one parked frame (survey D5). `rut_run` parks its `Vm` here when
/// the guest traps OutOfFuel with a live frame — `rut_resume` adds fuel
/// and continues THAT frame (the engine's own park/resume:
/// the pc and locals ride in the Vm), never a restart. `rut_run`
/// supersedes any parked frame (one Vm at a time; this module is
/// single-threaded) and `rut_drop_frame` retires it.
static mut PARKED: Option<rut_vm::interp::Vm> = None;

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for c in s.bytes() {
        if c == b'=' {
            break;
        }
        let v = match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Assemble a run envelope: accumulated output, trap, the entry-err
/// channel, budgets, and the parked flag (additive fields — older
/// consumers ignore them).
///
/// THE TWO CHANNELS STAY DISTINCT BY LAW (err-channel phase 3): `trap`
/// carries panics and budget/lifecycle failures — bugs, wiring drift,
/// the loud channel; `err` carries a RETURNED failure — a `main` that
/// returned the `(?T, err)` pair shape has its second component read
/// here when it is a non-empty `str` (the empty err is success, and any
/// other return shape is just data: no err). A soft-fail run reads
/// `"trap": null, "err": "<why>"`.
fn run_envelope(output: &[String], trap: Option<&str>, err: Option<&str>, fuel_used: u64, heap: u64, parked: bool) -> *mut u8 {
    let mut json = String::new();
    json.push_str("{\"output\":[");
    for (i, l) in output.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json_escape(l, &mut json);
    }
    json.push_str("],\"trap\":");
    match trap {
        Some(t) => json_escape(t, &mut json),
        None => json.push_str("null"),
    }
    json.push_str(",\"err\":");
    match err {
        Some(e) => json_escape(e, &mut json),
        None => json.push_str("null"),
    }
    json.push_str(",\"fuelUsed\":");
    json.push_str(&fuel_used.to_string());
    json.push_str(",\"heapBytes\":");
    json.push_str(&heap.to_string());
    json.push_str(",\"parked\":");
    json.push_str(if parked { "true" } else { "false" });
    json.push('}');
    envelope(json.as_bytes())
}

/// The entry-err convention read off a return value: the pair shape's
/// SECOND component, when it is a non-empty `str`. Anything else — nil,
/// a non-pair, a non-str second, the empty success err — is no err.
fn err_of_value(v: &rut_vm::Value) -> Option<String> {
    if let rut_vm::Value::Tuple(parts) = v {
        if parts.len() == 2 {
            if let rut_vm::Value::Str(e) = &parts[1] {
                if !e.is_empty() {
                    return Some(e.clone());
                }
            }
        }
    }
    None
}

#[no_mangle]
pub extern "C" fn rut_run(
    bin_ptr: *const u8,
    bin_len: usize,
    fuel: u64,
    heap_bytes: u64,
) -> *mut u8 {
    // a fresh run supersedes any parked frame — the old machine (and its
    // share of OUTPUT, reset just below) is retired first
    unsafe { PARKED = None };
    let bin = unsafe { core::slice::from_raw_parts(bin_ptr, bin_len) };
    let decode_result = rut_core::binary::decode(bin);
    let prog = match decode_result {
        Ok(p) => p,
        Err(e) => return run_envelope(&[], Some(&e), None, 0, 0, false),
    };
    if let Err(e) = rut_vm::verify::verify(&prog) {
        return run_envelope(&[], Some(&e), None, 0, 0, false);
    }
    *OUTPUT.lock().unwrap() = Some(Vec::new());
    let limits = rut_vm::interp::Limits {
        fuel: if fuel == 0 { None } else { Some(fuel) },
        heap_limit_bytes: if heap_bytes == 0 { None } else { Some(heap_bytes) },
        interrupt_every: 1024,
    };
    // the playground host's bindings, BEFORE the Vm: the logger routes
    // into the OUTPUT cell; calc's float fns ride rut-std. The installs
    // answer to the COMPILE-time mount snapshot (the ctx is owned; the
    // session died with the compile call) — the blanket-install lane:
    // this host ships no `http` (reqwest) and no `bench_cross`, the
    // rest merge inert unless the program mounts their pkg
    let ctx = CTX.lock().unwrap().clone().unwrap_or_default();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::logger::pkg(|s| {
        if let Ok(mut g) = OUTPUT.lock() {
            if let Some(v) = g.as_mut() {
                v.push(s.to_string());
            }
        }
    }));
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    // the nmap experiment's native key table (survey D6) — a program
    // only reaches it when it declares `use nmap_host::{...}` or a pkg
    // that does (`nmapset`); the CLI mounts it the same way
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    // the async host set: launch/abort/sleep bodies for the
    // `async_host` rows the playground offers in `compile_playground`
    hosts.install_host_pkg(&ctx, rut_std::async_host::pkg());
    // the string builder's bodies (the host strbuild pkg): a program
    // only reaches them when it declares `use strbuild::` (or `use
    // json::` — json's writer rides the builder)
    hosts.install_host_pkg(&ctx, rut_std::strbuild::pkg());
    let mut vm = match rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(hosts)
        .build()
    {
        Ok(vm) => vm,
        Err(e) => return run_envelope(&[], Some(&e.msg), None, 0, 0, false),
    };
    // the positional decode (RutValue): a `(?T, err)` main surfaces its
    // err here; every other shape (nil mains included) has none. A trap
    // NEVER fills err — the loud channel stays the loud channel.
    let out = vm.call::<_, RutValue>("main", ());
    // the async driving loop: drain the ready
    // queue, advance the virtual clock to the next sleep deadline — the
    // CLI's own loop (`rut run`): run what's ready, then jump `now` to
    // the earliest armed timer so sleep futures fire. Idle for programs
    // that launch nothing. The fuel budget parks (OutOfFuel) still
    // surface through `call`'s own result — a parked main takes the
    // parked-slot path below, untouched. A trapped async task stops the
    // drain (the loud channel answers on the envelope's next read).
    if out.is_ok() && !vm.is_running() {
        for _ in 0..1_000_000 {
            if vm.run_ready().is_err() {
                break;
            }
            match vm.next_deadline() {
                Some(d) => vm.set_now(d),
                None => {
                    if vm.pending_tasks() == 0 {
                        break;
                    }
                }
            }
        }
    }
    let (trap, parked, err) = match out {
        Ok(v) => (None, false, err_of_value(&v)),
        Err(t) => {
            let parked = t.kind == rut_vm::TrapKind::OutOfFuel && vm.is_running();
            (Some(t.name()), parked, None)
        }
    };
    // read the counters BEFORE the machine moves into the parked slot
    let fuel_used = vm.fuel_used;
    let heap = vm.heap_usage();
    if parked {
        unsafe { PARKED = Some(vm) };
    }
    let lines = OUTPUT.lock().unwrap().clone().unwrap_or_default();
    run_envelope(&lines, trap.as_deref(), err.as_deref(), fuel_used, heap, parked)
}

/// demo utilities (base64 in/out keeps the ABI tiny)
#[no_mangle]
pub extern "C" fn rut_run_b64(bin_b64_ptr: *const u8, bin_b64_len: usize, fuel: u64, heap: u64) -> *mut u8 {
    let s = unsafe { read_str(bin_b64_ptr, bin_b64_len) };
    match base64_decode(s) {
        Some(b) => rut_run(b.as_ptr(), b.len(), fuel, heap),
        None => envelope(b"{\"output\":[],\"trap\":\"bad base64\",\"err\":null,\"fuelUsed\":0,\"heapBytes\":0}"),
    }
}

// ---- resume (survey D5: the UI's Resume is REAL, not a re-run) ----

/// Continue the parked frame: add `extra_fuel` to the SAME machine and
/// re-enter the run loop at the parked pc. The output envelope carries
/// the ACCUMULATED lines of run+resumes (OUTPUT is never reset here —
/// only a fresh `rut_run` resets it), cumulative `fuelUsed`, and
/// `parked:true` when the frame parked again (it can run out of fuel
/// twice — each resume keeps going). `heap` is accepted for ABI
/// symmetry but the parked machine keeps the heap limit it was built
/// with — a resume cannot grow a budget retroactively. With no parked
/// frame this is LOUD: the trap names it, nothing silently re-runs.
#[no_mangle]
pub extern "C" fn rut_resume(extra_fuel: u64, _heap: u64) -> *mut u8 {
    let mut vm = match unsafe { PARKED.take() } {
        Some(vm) => vm,
        None => {
            return run_envelope(
                &[],
                Some("no parked frame — run first (resume continues, never restarts)"),
                None,
                0,
                0,
                false,
            )
        }
    };
    vm.add_fuel(extra_fuel);
    let out = vm.resume::<RutValue>();
    let (trap, parked, err) = match out {
        Ok(v) => (None, false, err_of_value(&v)),
        Err(t) => {
            let parked = t.kind == rut_vm::TrapKind::OutOfFuel && vm.is_running();
            (Some(t.name()), parked, None)
        }
    };
    let fuel_used = vm.fuel_used;
    let heap = vm.heap_usage();
    if parked {
        unsafe { PARKED = Some(vm) };
    }
    // the ACCUMULATED lines — run + every resume so far
    let lines = OUTPUT.lock().unwrap().clone().unwrap_or_default();
    run_envelope(&lines, trap.as_deref(), err.as_deref(), fuel_used, heap, parked)
}

/// Retire the parked frame (a case switch must not inherit the previous
/// case's machine). Returns 1 when a frame was dropped, 0 when none was
/// parked.
#[no_mangle]
pub extern "C" fn rut_drop_frame() -> u32 {
    match unsafe { PARKED.take() } {
        Some(_) => 1,
        None => 0,
    }
}

// ---- host tests: the ABI is testable natively (crate-type includes
// rlib; the same exports the wasm module offers run under `cargo test`)
// — the park/resume law is gated HERE, in the workspace gate, not only
// in the demo smoke. The statics (arena, OUTPUT, PARKED) are global, so
// every ABI test serializes on one lock.

#[cfg(test)]
mod tests {
    use super::*;

    const FUEL_DEMO: &str = r#"
use ink::{Logger};

entry fn main() {
    let log = Logger.new("case");
    let mut i = 0;
    while (true) {
        i += 1;
        if (i % 1000000 == 0) {
            log.info(f"tick {i}");
        }
    }
}
"#;

    const HELLO: &str = r#"
use ink::{Logger};

entry fn main() {
    let log = Logger.new("case");
    log.info("hello");
}
"#;

    /// every ABI test holds this — the statics are global
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn compile_case(src: &str) -> Vec<u8> {
        let out = compile_playground(src);
        assert!(
            out.diags.is_empty(),
            "test case must compile: {:?}",
            out.diags.iter().map(|d| &d.msg).collect::<Vec<_>>()
        );
        out.binary.expect("binary emitted")
    }

    /// copy bytes into the arena through the REAL `rut_alloc` and run
    fn run_json(bin: &[u8], fuel: u64, heap: u64) -> String {
        let ptr = rut_alloc(bin.len());
        assert!(!ptr.is_null(), "arena exhausted");
        unsafe { core::ptr::copy_nonoverlapping(bin.as_ptr(), ptr, bin.len()) };
        let res = rut_run(ptr, bin.len(), fuel, heap);
        read_envelope(res)
    }

    fn resume_json(extra_fuel: u64) -> String {
        let res = rut_resume(extra_fuel, 0);
        read_envelope(res)
    }

    /// [u32 LE len][json bytes] at the returned ptr
    fn read_envelope(ptr: *mut u8) -> String {
        unsafe {
            let len = u32::from_le(core::ptr::read(ptr as *const u32)) as usize;
            let bytes = core::slice::from_raw_parts(ptr.add(4), len);
            String::from_utf8(bytes.to_vec()).unwrap()
        }
    }

    fn field<'a>(json: &'a str, key: &str) -> &'a str {
        let needle = format!("\"{key}\":");
        let at = json.find(&needle).unwrap_or_else(|| panic!("no {key} in {json}"));
        let rest = &json[at + needle.len()..];
        // array values (output) scan to their MATCHING bracket — a naive
        // cut at the first comma would split ["a","b"]
        if rest.starts_with('[') {
            let mut depth = 0usize;
            let mut in_str = false;
            for (i, c) in rest.char_indices() {
                match c {
                    '"' => in_str = !in_str,
                    '[' if !in_str => depth += 1,
                    ']' if !in_str => {
                        depth -= 1;
                        if depth == 0 {
                            return &rest[..=i];
                        }
                    }
                    _ => {}
                }
            }
            panic!("unbalanced array for {key} in {json}");
        }
        let end = rest.find([',', '}']).unwrap();
        rest[..end].trim_matches('"')
    }

    #[test]
    fn run_parks_on_fuel_and_resume_continues_never_restarts() {
        let _g = lock();
        let bin = compile_case(FUEL_DEMO);

        // leg 1 at 12M: one tick (~10M ops to reach i=1M), parked
        let j1 = run_json(&bin, 12_000_000, 4 * 1024 * 1024);
        assert_eq!(field(&j1, "output"), "[\"tick 1000000\"]", "{j1}");
        assert_eq!(field(&j1, "trap"), "OutOfFuel", "{j1}");
        assert_eq!(field(&j1, "parked"), "true", "{j1}");
        let f1: u64 = field(&j1, "fuelUsed").parse().unwrap();
        assert!((11_000_000..13_000_000).contains(&f1), "{j1}");

        // resume with ZERO added fuel: re-traps at the parked pc — same
        // output, same spent counter. A restart could not burn 12M on a
        // zero budget; this pins the PARK.
        let j2 = resume_json(0);
        assert_eq!(field(&j2, "output"), "[\"tick 1000000\"]", "{j2}");
        assert_eq!(field(&j2, "trap"), "OutOfFuel", "{j2}");
        assert_eq!(field(&j2, "parked"), "true", "{j2}");
        let f2: u64 = field(&j2, "fuelUsed").parse().unwrap();
        assert!(f2 >= f1 && f2 < f1 + 1_000, "{j2} (f1={f1})");

        // resume with 16M more: the frame CONTINUES — tick 2000000 fires
        // (i never went back to 0; a restart's line would be a second
        // "tick 1000000"), tick 3000000 does not (16M can't reach it),
        // and the cumulative counter spans BOTH legs (>26M — a fresh
        // 16M run can never report that).
        let j3 = resume_json(16_000_000);
        assert_eq!(
            field(&j3, "output"),
            "[\"tick 1000000\",\"tick 2000000\"]",
            "{j3}"
        );
        assert_eq!(field(&j3, "trap"), "OutOfFuel", "{j3}");
        assert_eq!(field(&j3, "parked"), "true", "{j3}");
        let f3: u64 = field(&j3, "fuelUsed").parse().unwrap();
        assert!(f3 > 26_000_000, "{j3} (f1={f1})");
        assert!(!j3.contains("tick 3000000"), "{j3}");
    }

    #[test]
    fn clean_run_does_not_park_and_drop_retires_the_frame() {
        let _g = lock();
        unsafe { PARKED = None };
        let j = run_json(&compile_case(HELLO), 10_000_000, 4 * 1024 * 1024);
        assert_eq!(field(&j, "trap"), "null", "{j}");
        assert_eq!(field(&j, "parked"), "false", "{j}");
        assert_eq!(field(&j, "output"), "[\"hello\"]", "{j}");
        assert_eq!(unsafe { PARKED.is_some() }, false);
        assert_eq!(rut_drop_frame(), 0, "nothing was parked");
    }

    #[test]
    fn resume_without_a_frame_is_loud() {
        let _g = lock();
        unsafe { PARKED = None };
        let j = resume_json(10_000_000);
        assert!(j.contains("no parked frame"), "{j}");
        assert_eq!(field(&j, "parked"), "false", "{j}");
        assert_eq!(field(&j, "output"), "[]", "{j}");
    }

    #[test]
    fn fresh_run_supersedes_a_parked_frame_and_resets_output() {
        let _g = lock();
        let bin = compile_case(FUEL_DEMO);
        // park a frame
        let j1 = run_json(&bin, 12_000_000, 4 * 1024 * 1024);
        assert_eq!(field(&j1, "parked"), "true", "{j1}");
        // a fresh rut_run drops it and RESETS the accumulated output —
        // the new envelope carries exactly the new run's lines
        let j2 = run_json(&bin, 12_000_000, 4 * 1024 * 1024);
        assert_eq!(field(&j2, "output"), "[\"tick 1000000\"]", "{j2}");
        assert_eq!(field(&j2, "parked"), "true", "{j2}");
        assert_eq!(rut_drop_frame(), 1, "the second run's frame was parked");
    }

    // ---- the book lane's mounts: `use json::` / `use strbuild::`
    // resolve inside THIS host exactly like the CLI's loose-file set —
    // the docs run buttons must answer the same `rut run` does ----

    const BOOK_JSON: &str = r#"
use json::{ decodeJson, JsonI64 };
use strbuild::{ StringBuilder };
use ink::{ Logger };

entry fn main() {
    let log = Logger.new("book");
    let (n, e) = decodeJson<JsonI64>("42");
    if (e == nil) {
        let v: JsonI64 = n;
        let shown: i64 = v.get();
        log.info(f"n={shown}");
    }
    let mut b = StringBuilder.new();
    b.append("count: ");
    b.append(f"up to 42");
    log.info(b.build());
}
"#;

    #[test]
    fn the_book_lane_json_and_strbuild_mounts_resolve() {
        let _g = lock();
        unsafe { PARKED = None };
        let j = run_json(&compile_case(BOOK_JSON), 10_000_000, 4 * 1024 * 1024);
        assert_eq!(field(&j, "trap"), "null", "{j}");
        assert_eq!(
            field(&j, "output"),
            "[\"n=42\",\"count: up to 42\"]",
            "{j}"
        );
    }

    // the book's async block: launch + sleep must DRAIN like `rut run` —
    // the driving loop advances the virtual clock to each armed timer,
    // so the countdown completes (a clock that never moves parks after
    // the first sleep and the lines after it never fire)
    const BOOK_ASYNC: &str = r#"
use futures::{ launch_future, sleep };
use ink::{ Logger };

async fn countdown(log: Logger, n: u32) {
    let mut i = n;
    while (i > 0) {
        log.info(f"t-{i}");
        await sleep(500);
        i -= 1;
    }
    log.info("lift-off");
}

entry fn main() {
    let log = Logger.new("countdown");
    launch_future(countdown(log, 3));
}
"#;

    #[test]
    fn the_book_lane_async_lane_drains_to_completion() {
        let _g = lock();
        unsafe { PARKED = None };
        let j = run_json(&compile_case(BOOK_ASYNC), 10_000_000, 4 * 1024 * 1024);
        assert_eq!(field(&j, "trap"), "null", "{j}");
        assert_eq!(
            field(&j, "output"),
            "[\"t-3\",\"t-2\",\"t-1\",\"lift-off\"]",
            "{j}"
        );
    }

    // ---- the entry-err channel (err-channel phase 3): "err" beside
    // "trap". A (?T, err) main crosses under the ORIGINAL rule (the
    // nullable's element answers it — these cases compile ONLY with the
    // phase-3 `crosses_boundary` arm, so each is the gap-closed pin). ----

    const SOFT_FAIL: &str = r#"
entry fn main() -> (?str, str) {
    return (nil, "boom");
}
"#;

    const SOFT_OK: &str = r#"
entry fn main() -> (?str, str) {
    return ("the value", "");
}
"#;

    const SOFT_PANIC: &str = r#"
entry fn main() -> (?str, str) {
    panic("wiring drift");
}
"#;

    #[test]
    fn a_returned_err_rides_err_beside_a_null_trap() {
        let _g = lock();
        unsafe { PARKED = None };
        let j = run_json(&compile_case(SOFT_FAIL), 10_000_000, 4 * 1024 * 1024);
        assert_eq!(field(&j, "trap"), "null", "{j}");
        assert_eq!(field(&j, "err"), "boom", "{j}");
        assert_eq!(field(&j, "parked"), "false", "{j}");
    }

    #[test]
    fn an_empty_err_and_every_other_shape_surfaces_no_err() {
        let _g = lock();
        unsafe { PARKED = None };
        // the success leg: (value, "") — the value channel is live
        let j = run_json(&compile_case(SOFT_OK), 10_000_000, 4 * 1024 * 1024);
        assert_eq!(field(&j, "trap"), "null", "{j}");
        assert_eq!(field(&j, "err"), "null", "{j}");
        // the nil main — not the pair shape at all, just data
        let j = run_json(&compile_case(HELLO), 10_000_000, 4 * 1024 * 1024);
        assert_eq!(field(&j, "trap"), "null", "{j}");
        assert_eq!(field(&j, "err"), "null", "{j}");
        assert_eq!(field(&j, "output"), "[\"hello\"]", "{j}");
    }

    #[test]
    fn a_panic_never_fills_err_the_channels_never_mix() {
        let _g = lock();
        unsafe { PARKED = None };
        let j = run_json(&compile_case(SOFT_PANIC), 10_000_000, 4 * 1024 * 1024);
        assert_ne!(field(&j, "trap"), "null", "{j}");
        assert_eq!(field(&j, "err"), "null", "{j}");
    }
}
