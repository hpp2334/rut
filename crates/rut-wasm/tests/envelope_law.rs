//! The envelope lane's nameability law (D5). Plain `rut_compile`
//! DEFAULTS `main`'s manifest host-side to the offered lib names — so
//! every std head the page's programs name is declared, and the smoke's
//! cases keep compiling. `rut_compile_manifest(src, man)` is the page's
//! own door: the manifest it passes IS main's declaration — a use of a
//! mounted pkg outside that set is D5's fire (main is a hand offer, so
//! the diagnostic renders the host clause).
//!
//! Driven through the RAW ABI (the same exports the demo glue calls),
//! natively — the crate builds as rlib beside the cdylib.

/// One compile through the raw ABI; the JSON envelope back as a String.
fn compile_envelope(src: &str, man: Option<&str>) -> String {
    let (sp, sl) = (src.as_ptr(), src.len());
    let res_ptr = match man {
        None => rut_wasm::rut_compile(sp, sl),
        Some(m) => rut_wasm::rut_compile_manifest(sp, sl, m.as_ptr(), m.len()),
    };
    unsafe {
        // the ABI's envelope is [u32 LE length][bytes] at the returned
        // ptr — the static heap's offsets carry no alignment promise,
        // so the length reads unaligned (the JS glue's DataView reads
        // it the same way)
        let len = u32::from_le_bytes(std::ptr::read_unaligned(res_ptr as *const [u8; 4])) as usize;
        let bytes = std::slice::from_raw_parts(res_ptr.add(4), len);
        String::from_utf8_lossy(bytes).to_string()
    }
}

/// The `diags` array's messages, scraped from the envelope (the shape is
/// `{"diags":[{"start":N,"end":N,"msg":"..."},...],"ast":...}` — the
/// ABI's own writer, no serde on this path).
fn diag_msgs(envelope: &str) -> Vec<String> {
    let start = envelope.find("\"diags\":[").expect("diags key") + "\"diags\":[".len();
    let end = envelope[start..].find("],\"ast\"").expect("diags close") + start;
    let body = &envelope[start..end];
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("\"msg\":\"") {
        let after = &rest[i + "\"msg\":\"".len()..];
        let close = after.find("\"}").expect("msg close");
        out.push(after[..close].replace("\\\"", "\"").replace("\\\\", "\\"));
        rest = &after[close..];
    }
    out
}

/// Does the source compile clean through the lane?
fn compiles_clean(src: &str, man: Option<&str>) -> bool {
    let env = compile_envelope(src, man);
    let msgs = diag_msgs(&env);
    msgs.is_empty()
}

const INK_CALC_SRC: &str = r#"
use ink::{ Logger };
use calc::{ sqrt };
entry fn main() -> nil {
    let log = Logger("envelope");
    log.info(f"sqrt={sqrt(4.0)}");
}
"#;

#[test]
fn the_default_manifest_declares_the_offered_lib_names() {
    // plain rut_compile: the host-side default names EVERY offered lib,
    // so a use of ink + calc (both std heads) compiles clean — the
    // smoke's 26 programs ride exactly this lane
    assert!(
        compiles_clean(INK_CALC_SRC, None),
        "the default lane declares the std uses: {}",
        compile_envelope(INK_CALC_SRC, None)
    );
}

#[test]
fn a_manifest_lane_use_outside_the_set_is_d5_host_clause() {
    // the page's manifest names ONLY ink: calc stays mounted (the
    // world offers it) but undeclared by main → D5, and main is a hand
    // offer, so the host clause renders
    let env = compile_envelope(
        INK_CALC_SRC,
        Some(r#"{"name": "main", "deps": {"ink": {"path": "../ink"}}}"#),
    );
    let msgs = diag_msgs(&env);
    assert!(
        msgs.iter().any(|m| m.contains("cannot resolve `calc`")
            && m.contains("`calc` is mounted in this program (offered by the host)")
            && m.contains("`main` does not declare it")),
        "{msgs:?}"
    );
}

#[test]
fn a_manifest_lane_use_inside_the_set_compiles() {
    // the same manifest naming BOTH heads: declared, green — the door
    // never loosens beyond what the page spelled
    assert!(compiles_clean(
        INK_CALC_SRC,
        Some(
            r#"{"name": "main", "deps": {"ink": {"path": "../ink"}, "calc": {"path": "../calc"}}}"#
        ),
    ));
}
