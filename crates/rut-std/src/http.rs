//! The std HTTP lane's bodies (the rut/http plan): the four crossings
//! behind `rut/http_host`'s surface, wrapped by `rut/http`'s
//! `Response` class (the ink pattern) —
//!
//! ```rut
//! pub host fn http_get(url: str) -> opaque;
//! pub host fn http_status(r: opaque) -> i32;   // 0 = transport error
//! pub host fn http_err(r: opaque) -> ?str;     // nil unless status 0
//! pub host fn http_body(r: opaque) -> bytes;   // the body octets
//! ```
//!
//! The response handle is an `opaque` payload owning
//! `{ status, err, body }` — status `0` is RESERVED for transport
//! failure (0 is never a real HTTP status), the failure text rides
//! `err`, the body is empty on failure. An HTTP status (any, including
//! 4xx/5xx) is NOT a transport failure: `err` stays nil.
//!
//! The readback rows answer through the TYPED answer lanes (the
//! legal-host-returns phase): `Ret for Option<String>` mints the
//! `?str` opt box nil-flattened, `Ret for Vec<u8>` copies the octets
//! into a fresh `bytes` cell — each answer's cell is owned by the
//! CALLER's register, so the payload carries no claim bookkeeping (the
//! any-lane era's store-keeps-its-own law retired with the lane).
//!
//! Two lanes install these bodies: [`install_std_http_with`] rides an
//! injectable transport closure (the fixture lane — tests key it on
//! the exact URL and never touch the network); [`install_std_http`]
//! is the reqwest lane (a shared blocking client). The `http` cargo
//! feature is DEFAULT-OFF — reqwest-blocking does not build on
//! wasm32-unknown-unknown, so only native embedders opt in.

use rut_vm::interp::{HostRegistry, Vm};
use rut_vm::{HostPayload, Opaque, OpaqueRef, Trap};

/// The response handle's payload — the plan's `{ status, err, body }`
/// triple. No finalize hook: the readbacks answer owned Rust values
/// (`Option<String>`/`Vec<u8>`) whose crossing cells the caller owns.
pub struct HttpResponse {
    /// the HTTP status word — 0 is RESERVED for transport failure
    status: u16,
    /// the transport-failure text; always `None` unless status == 0
    err: Option<String>,
    /// the body octets; empty on transport failure
    body: Vec<u8>,
}

impl HostPayload for HttpResponse {}

/// Install the `http_host` bodies over an injectable transport: `f`
/// maps a URL to `(status, body)` — or `Err(message)` for a transport
/// failure, which lands as status 0 with the message in `err` and an
/// empty body. The fixture lane: tests key the closure on the exact
/// URL and answer recorded payloads, zero network.
pub fn install_std_http_with<F>(hosts: &mut HostRegistry, f: F)
where
    F: Fn(&str) -> Result<(u16, Vec<u8>), String> + 'static,
{
    // the ONE transport crossing: the handle mints here, the readbacks
    // ride the payload
    rut_vm::register!(
        hosts,
        "http_host::http_get",
        (&str,) -> OpaqueRef,
        move |vm: &mut Vm, url: &str| -> Result<OpaqueRef, Trap> {
            let (status, err, body) = match f(url) {
                Ok((status, body)) => (status, None, body),
                Err(message) => (0, Some(message), Vec::new()),
            };
            let b = Opaque::alloc_hosted(
                vm,
                HttpResponse { status, err, body },
            )?;
            Ok(b.handle().clone())
        },
    );
    rut_vm::register!(
        hosts,
        "http_host::http_status",
        (Opaque<HttpResponse>,) -> i32,
        |_vm: &mut Vm, r: Opaque<HttpResponse>| -> Result<i32, Trap> {
            r.with(|resp| resp.status as i32)
        },
    );
    rut_vm::register!(
        hosts,
        "http_host::http_err",
        (Opaque<HttpResponse>,) -> Option<String>,
        |_vm: &mut Vm, r: Opaque<HttpResponse>| -> Result<Option<String>, Trap> {
            Ok(r.with(|resp| resp.err.clone())?)
        },
    );
    rut_vm::register!(
        hosts,
        "http_host::http_body",
        (Opaque<HttpResponse>,) -> Vec<u8>,
        |_vm: &mut Vm, r: Opaque<HttpResponse>| -> Result<Vec<u8>, Trap> {
            Ok(r.with(|resp| resp.body.clone())?)
        },
    );
}

/// Install the reqwest lane over [`install_std_http_with`]: one shared
/// blocking client (this UA, redirects on — the default policy's ten
/// hops; gzip/brotli arrive with the cargo features), transport
/// failure mapped to `Err(message)` — the status-0 lane. Requires the
/// `http` cargo feature.
#[cfg(feature = "http")]
pub fn install_std_http(hosts: &mut HostRegistry) {
    let client = reqwest::blocking::Client::builder()
        .user_agent("rgh/0.1 (+https://github.com/rut)")
        .redirect(reqwest::redirect::Policy::default())
        .build()
        .expect("the std http client builds");
    install_std_http_with(hosts, move |url| {
        let response = client.get(url).send().map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let body = response.bytes().map_err(|e| e.to_string())?.to_vec();
        Ok((status, body))
    });
}

#[cfg(all(test, feature = "http"))]
mod tests {
    use super::*;
    use std::rc::Rc;

    /// this test's host pkg — the mounted decl surface the bodies must
    /// match (RFC 0025)
    const PKG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/http_host");

    /// the probe: one URL in, the readback triple out — the host
    /// compares Rust-side (the 02-digest oracle pattern)
    const SRC: &str = r#"
use http_host::{ http_get, http_status, http_err, http_body };

entry fn probe(url: str) -> (i32, ?str, bytes) {
    let r: opaque = http_get(url);
    return (http_status(r), http_err(r), http_body(r));
}

entry fn twice(url: str) -> (bytes, bytes) {
    let r: opaque = http_get(url);
    return (http_body(r), http_body(r));
}
"#;

    /// Boot the probe over the fixture lane (the rut-cli nmap test's
    /// shape): mount std-core + `rut/http_host`, compile, verify, bind
    /// the fixture bodies, join — `verify_against` runs pre-boot, so a
    /// decl↔body drift fails loudly in every test below.
    fn boot(fixture: impl Fn(&str) -> Result<(u16, Vec<u8>), String> + 'static) -> Vm {
        let mut session = rut_driver::Session::new();
        rut_driver::mount_std_core(&mut session);
        rut_driver::mount_dir(&mut session, std::path::Path::new(PKG_DIR))
            .expect("mount http_host");
        let expected = session.expected_host_fns();
        session
            .register_module(
                "app",
                rut_driver::Module {
                    spec: "app".into(),
                    source: Some(SRC.into()),
                    ..Default::default()
                },
            )
            .expect("register the probe");
        let g = rut_driver::compile_graph(&session, "app");
        assert!(
            g.diags.is_empty(),
            "{}",
            g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
        );
        let prog = g.program.expect("compile");
        rut_vm::verify::verify(&prog).unwrap();
        let limits = rut_vm::interp::Limits {
            fuel: Some(1_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        };
        // bindings BEFORE the Vm (RFC 0025): install + contract + boot
        let mut hosts = rut_vm::interp::HostRegistry::new();
        install_std_http_with(&mut hosts, fixture);
        hosts.verify_against(&expected); // the decl ↔ the bodies
        rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts)
            .unwrap()
    }

    #[test]
    fn fixture_semantics_ok_status_body_and_url_passthrough() {
        let seen = Rc::new(std::cell::RefCell::new(String::new()));
        let seen2 = seen.clone();
        let mut vm = boot(move |url| {
            seen2.borrow_mut().push_str(url);
            Ok((200, b"hello rut".to_vec()))
        });
        let got: (i32, Option<String>, Vec<u8>) = vm.call("probe", ("fixture://ok",)).unwrap();
        assert_eq!(got.0, 200);
        assert_eq!(got.1, None, "a transport success carries no err text");
        assert_eq!(got.2, b"hello rut");
        assert_eq!(seen.borrow().as_str(), "fixture://ok", "the url crosses verbatim");
        // repeated reads answer fresh cells, same octets
        let (a, b): (Vec<u8>, Vec<u8>) = vm.call("twice", ("fixture://ok",)).unwrap();
        assert_eq!(a, b"hello rut");
        assert_eq!(b, b"hello rut");
    }

    #[test]
    fn transport_failure_reserves_status_zero() {
        let mut vm = boot(|_| Err("boom: the transport failed".into()));
        let got: (i32, Option<String>, Vec<u8>) = vm.call("probe", ("fixture://bad",)).unwrap();
        assert_eq!(got.0, 0, "0 is RESERVED for transport failure (never a real status)");
        assert_eq!(got.1.as_deref(), Some("boom: the transport failed"));
        assert!(got.2.is_empty(), "the body is empty on transport failure");
    }

    #[test]
    fn http_error_statuses_are_not_transport_failures() {
        // 0-reserved law's other side: a real status (4xx included)
        // keeps err nil and the body intact — the caller maps it
        let mut vm = boot(|_| Ok((404, b"nope".to_vec())));
        let got: (i32, Option<String>, Vec<u8>) = vm.call("probe", ("fixture://404",)).unwrap();
        assert_eq!(got.0, 404);
        assert_eq!(got.1, None, "an HTTP status is not a transport failure");
        assert_eq!(got.2, b"nope");
    }

    #[test]
    fn answer_cells_close_at_frame_exit() {
        // every readback mints a cell the CALLER's register owns (the
        // typed answer lanes); when the probe's frame retires the
        // balance must close on the base — on the ok lane and the
        // failure lane alike
        let mut vm = boot(|url| match url {
            "fixture://bad" => Err("boom".into()),
            _ => Ok((200, b"payload".to_vec())),
        });
        let base = vm.heap_usage();
        let _: (i32, Option<String>, Vec<u8>) = vm.call("probe", ("fixture://ok",)).unwrap();
        assert_eq!(vm.heap_usage(), base, "the ok lane closes on the base");
        let _: (i32, Option<String>, Vec<u8>) = vm.call("probe", ("fixture://bad",)).unwrap();
        assert_eq!(vm.heap_usage(), base, "the failure lane closes on the base");
    }

    #[test]
    fn the_decl_rows_join_both_lanes() {
        let mut session = rut_driver::Session::new();
        rut_driver::mount_std_core(&mut session);
        rut_driver::mount_dir(&mut session, std::path::Path::new(PKG_DIR))
            .expect("mount http_host");
        let expected = session.expected_host_fns();
        // the fixture lane
        let mut hosts = rut_vm::interp::HostRegistry::new();
        install_std_http_with(&mut hosts, |_| Ok((200, Vec::new())));
        hosts.verify_against(&expected); // panics on drift
        // the reqwest lane binds the same rows — no network at bind time
        let mut hosts = rut_vm::interp::HostRegistry::new();
        install_std_http(&mut hosts);
        hosts.verify_against(&expected);
    }
}
