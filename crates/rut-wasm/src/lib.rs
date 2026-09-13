//! rut-wasm — the demo page's compile/run surface (RFC 0041 §3), mirrored
//! by `demo/src/wasm/rut-api.d.ts`.
//!
//! Raw ABI over wasm32 linear memory (no wasm-bindgen glue):
//!   rut_alloc(len) -> ptr                      bump allocation
//!   rut_compile(src_ptr, src_len) -> result    JSON envelope, base64 binary
//!   rut_run(bin_ptr, bin_len, fuel, heap) -> result
//!   rut_result_len() / rut_result_ptr()        read the last envelope
//! Every result is [u32 little-endian length][bytes] at the returned ptr.

#![allow(static_mut_refs)]

use std::cell::RefCell;
use std::rc::Rc;

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

#[no_mangle]
pub extern "C" fn rut_compile(src_ptr: *const u8, src_len: usize) -> *mut u8 {
    let src = unsafe { read_str(src_ptr, src_len) };
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "main");
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

#[no_mangle]
pub extern "C" fn rut_run(
    bin_ptr: *const u8,
    bin_len: usize,
    fuel: u64,
    heap_bytes: u64,
) -> *mut u8 {
    let bin = unsafe { core::slice::from_raw_parts(bin_ptr, bin_len) };
    let mut json = String::new();
    let decode_result = rut_core::binary::decode(bin);
    let prog = match decode_result {
        Ok(p) => p,
        Err(e) => {
            json.push_str("{\"output\":[],\"trap\":");
            json_escape(&e, &mut json);
            json.push_str(",\"fuelUsed\":0,\"heapBytes\":0}");
            return envelope(json.as_bytes());
        }
    };
    if let Err(e) = rut_vm::verify::verify(&prog) {
        json.push_str("{\"output\":[],\"trap\":");
        json_escape(&e, &mut json);
        json.push_str(",\"fuelUsed\":0,\"heapBytes\":0}");
        return envelope(json.as_bytes());
    }
    *OUTPUT.lock().unwrap() = Some(Vec::new());
    let limits = rut_vm::interp::Limits {
        fuel: if fuel == 0 { None } else { Some(fuel) },
        heap_limit_bytes: if heap_bytes == 0 { None } else { Some(heap_bytes) },
        interrupt_every: 1024,
    };
    let mut vm = match rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default()) {
        Ok(vm) => vm,
        Err(t) => {
            json.push_str("{\"output\":[],\"trap\":");
            json_escape(&t.name(), &mut json);
            json.push_str(",\"fuelUsed\":0,\"heapBytes\":0}");
            return envelope(json.as_bytes());
        }
    };
    rut_std::logger::install_std_log(&mut vm, |s| {
        if let Ok(mut g) = OUTPUT.lock() {
            if let Some(v) = g.as_mut() {
                v.push(s.to_string());
            }
        }
    });
    let (trap, fuel_used) = match vm.call("main", &[]) {
        Ok(_) => (None, vm.fuel_used),
        Err(t) => (Some(t.name()), vm.fuel_used),
    };
    let heap = vm.heap_usage();
    let lines = OUTPUT.lock().unwrap().take().unwrap_or_default();
    json.push_str("{\"output\":[");
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json_escape(l, &mut json);
    }
    json.push_str("],\"trap\":");
    match trap {
        Some(t) => json_escape(&t, &mut json),
        None => json.push_str("null"),
    }
    json.push_str(",\"fuelUsed\":");
    json.push_str(&fuel_used.to_string());
    json.push_str(",\"heapBytes\":");
    json.push_str(&heap.to_string());
    json.push('}');
    envelope(json.as_bytes())
}

/// demo utilities (RFC 0041 §3 OQ-1: base64 in/out keeps the ABI tiny)
#[no_mangle]
pub extern "C" fn rut_run_b64(bin_b64_ptr: *const u8, bin_b64_len: usize, fuel: u64, heap: u64) -> *mut u8 {
    let s = unsafe { read_str(bin_b64_ptr, bin_b64_len) };
    match base64_decode(s) {
        Some(b) => rut_run(b.as_ptr(), b.len(), fuel, heap),
        None => envelope(b"{\"output\":[],\"trap\":\"bad base64\",\"fuelUsed\":0,\"heapBytes\":0}"),
    }
}
