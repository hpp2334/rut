//! The envelope lane's nameability law (D5). Plain `compile`
//! DEFAULTS `main`'s manifest host-side to the offered lib names — so
//! every std head the page's programs name is declared, and the smoke's
//! cases keep compiling. `compile_manifest(src, man)` is the page's
//! own door: the manifest it passes IS main's declaration — a use of a
//! mounted pkg outside that set is D5's fire (main is a hand offer, so
//! the diagnostic renders the host clause).
//!
//! Driven through the SAME methods the wasm class offers, natively —
//! the crate builds as rlib beside the cdylib.

/// One compile through the engine; the JSON envelope back as a String.
fn compile_envelope(src: &str, man: Option<&str>) -> String {
    let engine = rut_wasm::RutEngine::new();
    match man {
        None => engine.compile(src),
        Some(m) => engine.compile_manifest(src, m),
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
    // plain compile: the host-side default names EVERY offered lib,
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
