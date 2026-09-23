//! rut-wasm — the demo page's compile/run surface (RFC 0041 §3), mirrored
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

use std::cell::RefCell;
use std::rc::Rc;

// the positional driver decode (`Ret for Value`) — the run envelope's
// entry-err reader; aliased so the host's own JSON never collides
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

/// The playground's library mounts (§0.9 of the host-pkgs plan): wasm
/// has no filesystem, so the toolchain libs this host ships are
/// EMBEDDED — `rt`'s and `nmap_host`'s host surfaces lowered from their
/// `.d.rut`s, `ink`, `pouch` and `nmapset` as in-memory source. This
/// host's choice, not the engine's: the driver knows none of these
/// names. (`nmap_host` + `nmapset` are the survey D6 amendment — the
/// demo's map lane; `install_std_nmap` binds the crossings in `rut_run`.)
fn compile_playground(src: &str) -> rut_driver::CompileOutput {
    let mut session = rut_driver::Session::new();
    rut_driver::mount_std(&mut session); // core + calc
    let mut rt = rut_driver::lower_decl_module(
        include_str!("../../../rut/rt/rt.d.rut"),
        "rt.d.rut",
    )
    .expect("the rt surface is valid");
    rt.host_scope = Some("rt:log".to_string()); // rut-std's registration prefix
    session.register_module("rt", rt).expect("mount rt");
    // the nmap lane (survey D6): the host surface lowers exactly like
    // `rt` — its registration scope is the default (the module spec
    // `nmap_host`), the prefix `install_std_nmap` registers under
    let nmap_host = rut_driver::lower_decl_module(
        include_str!("../../../rut/nmap_host/nmap.d.rut"),
        "nmap.d.rut",
    )
    .expect("the nmap_host surface is valid");
    session
        .register_module("nmap_host", nmap_host)
        .expect("mount nmap_host");
    session
        .register_module(
            "ink",
            rut_driver::Module {
                source: Some(include_str!("../../../rut/ink/ink.rut").to_string()),
                inline: true,
                ..Default::default()
            },
        )
        .expect("mount ink");
    session
        .register_module(
            "pouch",
            rut_driver::Module {
                source: Some(include_str!("../../../rut/pouch/pouch.rut").to_string()),
                ..Default::default()
            },
        )
        .expect("mount pouch");
    // `nmapset` — inline like `ink` (a generic-class module is
    // source-inlined into its consumer); its `use nmap_host::` resolves
    // against the mounted surface above
    session
        .register_module(
            "nmapset",
            rut_driver::Module {
                source: Some(include_str!("../../../rut/nmapset/nmapset.rut").to_string()),
                inline: true,
                ..Default::default()
            },
        )
        .expect("mount nmapset");
    rut_driver::compile_module_in(&mut session, src, rut_parser::Mode::Impl, "main")
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

/// The one parked frame (survey D5). `rut_run` parks its `Vm` here when
/// the guest traps OutOfFuel with a live frame — `rut_resume` adds fuel
/// and continues THAT frame (the engine's own park/resume, RFC 0034 §4:
/// the pc and locals ride in the Vm), never a restart. `rut_run`
/// supersedes any parked frame (one Vm at a time; this module is
/// single-threaded, RFC 0034) and `rut_drop_frame` retires it.
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
    // the playground host's bindings, BEFORE the Vm (RFC 0025): the
    // logger routes into the OUTPUT cell; calc's float fns ride rut-std
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::logger::install_std_log(&mut hosts, |s| {
        if let Ok(mut g) = OUTPUT.lock() {
            if let Some(v) = g.as_mut() {
                v.push(s.to_string());
            }
        }
    });
    rut_std::math::install_std_math(&mut hosts);
    // the nmap experiment's native key table (survey D6) — a program
    // only reaches it when it declares `use nmap_host::{...}` or a pkg
    // that does (`nmapset`); the CLI mounts it the same way
    rut_std::nmap::install_std_nmap(&mut hosts);
    let mut vm = match rut_vm::interp::Vm::new(
        Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        hosts,
    ) {
        Ok(vm) => vm,
        Err(t) => return run_envelope(&[], Some(&t.name()), None, 0, 0, false),
    };
    // the positional decode (RutValue): a `(?T, err)` main surfaces its
    // err here; every other shape (nil mains included) has none. A trap
    // NEVER fills err — the loud channel stays the loud channel.
    let out = vm.call::<_, RutValue>("main", ());
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

/// demo utilities (RFC 0041 §3 OQ-1: base64 in/out keeps the ABI tiny)
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

pub fn main() {
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

pub fn main() {
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
