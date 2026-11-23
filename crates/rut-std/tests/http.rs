#![cfg(feature = "http")] // preserves today's exact gating (resolver 2:
// only `-p rut-std --features http` builds these)
//! The http lane's white-box suite, moved verbatim out of
//! `src/http.rs`: the fixture lane driven end to end through the rut
//! face — send resolving at headers, the body/stream one-shot law,
//! mid-read death as data, the builder's verbs and headers, the
//! decl-row join over both lanes, the real reqwest lane over a local
//! TCP server.

use rut_driver::MountedBody;
use rut_std::http::{pkg, pkg_with, FixtureReply, HttpFixture};
use rut_std::{async_host, logger, strbuild};
use rut_vm::interp::Vm;
use std::rc::Rc;

/// this test's host pkgs — the mounted decl surface the bodies must
/// match
const PKG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut");

/// the probe: the FACE (the `rut/http` classes), driven as an async
/// free fn over the launcher — the same shape the rgh brain rides.
/// The awaits deliver (the answer lane): send at headers, body as
/// the drain, stream reads per chunk.
const SRC: &str = r#"
use http::{ HttpClient, ClientQueryMethod };
use http_host::net::{ http_status, http_err, http_read_err };
use futures::launch_future;
use ink_host::{ create_logger, logger_log };

async fn probe(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let resp = await client.post(url).body("hi".encode()).build().send();
    logger_log(log, 2, f"status={resp.status()}");
    // the readbacks are ?str — nil-check, then deref, then log (the
    // rgh deref pattern; a ?str never interpolates)
    let terr = resp.transport_error();
    if (terr == nil) {
        logger_log(log, 2, "err=nil");
    } else {
        let t: str = terr;
        logger_log(log, 2, f"err={t}");
    }
    let rerr = resp.read_error();
    if (rerr == nil) {
        logger_log(log, 2, "read-err=nil");
    } else {
        let t: str = rerr;
        logger_log(log, 2, f"read-err={t}");
    }
    let b = await resp.body();
    logger_log(log, 2, f"body={b.len()}");
}

async fn streamed(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let resp = await client.get(url).build().send();
    let s = resp.byte_stream();
    let total: i32 = 0;
    let failed = false;
    while (true) {
        let c = await s.next();
        if (c == nil) {
            let e = s.error();
            if (e != nil) { failed = true; }
            break;
        }
        let chunk: bytes = c;
        total += chunk.len();
    }
    logger_log(log, 2, f"stream-total={total}");
    logger_log(log, 2, f"stream-failed={failed}");
}

entry fn boot(url: str) -> nil {
    let log = create_logger("t");
    launch_future(probe(log, url));
}

entry fn boot_stream(url: str) -> nil {
    let log = create_logger("t");
    launch_future(streamed(log, url));
}
"#;

fn boot(
    fixture: impl Fn(&str, &str, &str, &[u8]) -> Result<FixtureReply, String> + 'static,
) -> (Vm, Rc<std::cell::RefCell<Vec<String>>>, HttpFixture) {
    let mut world = rut_native::dir_pkgs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"),
    )
    .expect("the async pair walks")
    .pkgs;
    for d in ["ink_host", "http_host", "http"] {
        world.extend(
            rut_native::dir_pkgs(&std::path::Path::new(PKG_DIR).join(d))
                .unwrap_or_else(|e| panic!("mount {d}: {e}"))
                .pkgs,
        );
    }
    world.push(
        rut_driver::Mounted::source("app", SRC)
            // the app declares the heads it names (the nameability
            // law — mounted-but-undeclared is D5)
            .manifest(rut_driver::Manifest::lib(
                "app",
                &["http", "http_host", "futures", "async_host", "ink_host"],
            )),
    );
    let compiled = rut_driver::Walk {
        pkgs: world.clone(),
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
    .entrypoint("app")
    .compile()
    .unwrap();
    assert!(
        compiled.graph.diags.is_empty(),
        "{}",
        compiled
            .graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let prog = compiled.graph.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let ctx = rut_driver::host_pkg_ctx(&world);
    let mut hosts = rut_vm::interp::HostRegistry::new();
    let sink2 = sink.clone();
    hosts.install_host_pkg(
        &ctx,
        logger::pkg(move |m| sink2.borrow_mut().push(m.to_string())),
    );
    hosts.install_host_pkg(&ctx, async_host::pkg());
    // the http face's responses build through the strbuild pkg's
    // StringBuilder — its `strbuild_host` rows are declared here, so
    // the bodies install alongside the async set
    hosts.install_host_pkg(&ctx, strbuild::pkg());
    let (http_pkg, fx) = pkg_with(fixture);
    hosts.install_host_pkg(&ctx, http_pkg);
    hosts.verify_against(&ctx.flatten()); // the decl ↔ the bodies
    let vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hosts(hosts)
        .build()
        .unwrap();
    (vm, sink, fx)
}

/// the driving loop over the virtual clock: drain the ready queue,
/// settle the fixture's dues, advance to the next due/deadline
fn run_loop(vm: &mut Vm, fx: &HttpFixture, cap: usize) {
    for _ in 0..cap {
        vm.run_ready().expect("run_ready");
        if vm.pending_tasks() == 0 && !fx.has_pending() {
            return;
        }
        if fx.settle(vm.now_ms()) {
            continue;
        }
        let next = [fx.next_due(), vm.next_deadline()]
            .into_iter()
            .flatten()
            .filter(|&d| d > vm.now_ms())
            .min();
        match next {
            Some(d) => vm.set_now(d),
            None => vm.set_now(vm.now_ms() + 1),
        }
    }
    panic!(
        "the driving loop stalled: {} task(s) pending",
        vm.pending_tasks()
    );
}

#[test]
fn the_face_sends_posts_and_drains() {
    let seen = Rc::new(std::cell::RefCell::new(String::new()));
    let seen2 = seen.clone();
    let (mut vm, sink, fx) = boot(move |method, url, _h, body| {
        seen2.borrow_mut().push_str(&format!(
            "{method} {url} body={}\n",
            String::from_utf8_lossy(body)
        ));
        if url == "fixture://ok" {
            Ok(FixtureReply::ok(200, b"hello rut"))
        } else {
            Err(format!("rgh-test: unexpected url: {url}"))
        }
    });
    vm.call::<_, ()>("boot", ("fixture://ok",)).unwrap();
    run_loop(&mut vm, &fx, 100);
    assert_eq!(
        *sink.borrow(),
        vec!["status=200", "err=nil", "read-err=nil", "body=9"],
        "send resolves at headers, body drains through the answer lane"
    );
    assert_eq!(
        seen.borrow().as_str(),
        "POST fixture://ok body=hi\n",
        "the builder crossed method+url+body verbatim"
    );
}

#[test]
fn transport_failure_reserves_status_zero_and_degrades() {
    let (mut vm, sink, fx) = boot(|_m, url, _h, _b| Err(format!("boom: {url} failed")));
    vm.call::<_, ()>("boot", ("fixture://bad",)).unwrap();
    run_loop(&mut vm, &fx, 100);
    assert_eq!(
        *sink.borrow(),
        vec![
            "status=0",
            "err=boom: fixture://bad failed",
            "read-err=nil",
            "body=0"
        ],
        "status 0 + the err text; the body lane degrades to empty"
    );
}

#[test]
fn http_error_statuses_are_not_transport_failures() {
    let (mut vm, sink, fx) = boot(|_m, _u, _h, _b| Ok(FixtureReply::ok(404, b"nope")));
    vm.call::<_, ()>("boot", ("fixture://404",)).unwrap();
    run_loop(&mut vm, &fx, 100);
    assert_eq!(
        *sink.borrow(),
        vec!["status=404", "err=nil", "read-err=nil", "body=4"],
        "a real status (4xx included) keeps err nil and the body intact"
    );
}

#[test]
fn the_stream_lane_walks_deterministic_chunks_to_eof() {
    let (mut vm, sink, fx) = boot(|_m, _u, _h, _b| {
        Ok(FixtureReply::chunked(
            200,
            vec![b"ab".to_vec(), b"cde".to_vec(), b"f".to_vec()],
        ))
    });
    vm.call::<_, ()>("boot_stream", ("fixture://chunks",))
        .unwrap();
    run_loop(&mut vm, &fx, 100);
    assert_eq!(
        *sink.borrow(),
        vec!["stream-total=6", "stream-failed=false"],
        "chunk per next, EOF nil, the sticky err stays clean"
    );
}

#[test]
fn a_mid_read_death_is_data_never_a_trap() {
    let (mut vm, sink, fx) = boot(|_m, _u, _h, _b| {
        Ok(FixtureReply::failing(
            200,
            vec![b"ab".to_vec(), b"cd".to_vec(), b"ef".to_vec()],
            2,
            "connection reset by peer",
        ))
    });
    vm.call::<_, ()>("boot_stream", ("fixture://die",)).unwrap();
    run_loop(&mut vm, &fx, 100);
    assert_eq!(
        *sink.borrow(),
        vec!["stream-total=4", "stream-failed=true"],
        "two chunks, then nil; the sticky read err is set (both readbacks read one fact)"
    );
}

#[test]
fn the_body_lane_answers_the_short_drain_after_a_mid_read_death() {
    let src = r#"
use http::HttpClient;
use futures::launch_future;
use ink_host::{ create_logger, logger_log };

async fn probe(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let resp = await client.get(url).build().send();
    let b = await resp.body();
    logger_log(log, 2, f"drain={b.len()}");
    logger_log(log, 2, f"read-err={resp.read_error()}");
}

entry fn boot(url: str) -> nil {
    let log = create_logger("t");
    launch_future(probe(log, url));
}
"#;
    // splice the probe source over the default one and re-boot by hand
    let mut world = rut_native::dir_pkgs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"),
    )
    .expect("the async pair walks")
    .pkgs;
    let tree = std::path::Path::new(PKG_DIR);
    for d in ["ink_host", "http_host", "http"] {
        world.extend(
            rut_native::dir_pkgs(&tree.join(d))
                .unwrap_or_else(|e| panic!("mount {d}: {e}"))
                .pkgs,
        );
    }
    world.push(
        rut_driver::Mounted::source("app", src)
            // the app declares the heads it names (the nameability
            // law — mounted-but-undeclared is D5)
            .manifest(rut_driver::Manifest::lib(
                "app",
                &["http", "http_host", "futures", "async_host", "ink_host"],
            )),
    );
    let compiled = rut_driver::Walk {
        pkgs: world.clone(),
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
    .entrypoint("app")
    .compile()
    .unwrap();
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags
    );
    let prog = compiled.graph.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let ctx = rut_driver::host_pkg_ctx(&world);
    let mut hosts = rut_vm::interp::HostRegistry::new();
    let sink2 = sink.clone();
    hosts.install_host_pkg(
        &ctx,
        logger::pkg(move |m| sink2.borrow_mut().push(m.to_string())),
    );
    hosts.install_host_pkg(&ctx, async_host::pkg());
    // the http face's responses build through the strbuild pkg's
    // StringBuilder — its `strbuild_host` rows are declared here, so
    // the bodies bind alongside the async set
    hosts.install_host_pkg(&ctx, strbuild::pkg());
    let (http_pkg, fx) = pkg_with(|_m, _u, _h, _b| {
        Ok(FixtureReply::failing(
            200,
            vec![b"xy".to_vec(), b"zw".to_vec()],
            1,
            "eof in chunk",
        ))
    });
    hosts.install_host_pkg(&ctx, http_pkg);
    hosts.verify_against(&ctx.flatten());
    let limits = rut_vm::interp::Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hosts(hosts)
        .build()
        .unwrap();
    vm.call::<_, ()>("boot", ("fixture://short",)).unwrap();
    run_loop(&mut vm, &fx, 100);
    assert_eq!(
        *sink.borrow(),
        vec!["drain=2", "read-err=eof in chunk"],
        "one chunk arrived, then the wire died — the drain answers the short body"
    );
}

#[test]
fn the_one_shot_law_degrades_second_takers() {
    let src = r#"
use http::HttpClient;
use futures::launch_future;
use ink_host::{ create_logger, logger_log };

async fn probe(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let resp = await client.get(url).build().send();
    // body first, THEN a stream mint: the mint degrades to a dead
    // reader — immediate EOF, empty error
    let b = await resp.body();
    logger_log(log, 2, f"body={b.len()}");
    let s = resp.byte_stream();
    let c = await s.next();
    if (c == nil) {
        logger_log(log, 2, "late-next-nil=true");
    } else {
        logger_log(log, 2, "late-next-nil=false");
    }
    let le = s.error();
    if (le == nil) {
        logger_log(log, 2, "late-err=nil");
    } else {
        let t: str = le;
        logger_log(log, 2, f"late-err={t}");
    }
}

// the other order: stream first, then body — the drain degrades to
// empty
async fn probe2(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let resp = await client.get(url).build().send();
    let s = resp.byte_stream();
    let c = await s.next();
    let n: i32 = -1;
    if (c != nil) {
        let v: bytes = c;
        n = v.len();
    }
    logger_log(log, 2, f"first-next={n}");
    let b = await resp.body();
    logger_log(log, 2, f"late-body={b.len()}");
}

entry fn boot(url: str) -> nil {
    let log = create_logger("t");
    launch_future(probe(log, url));
    launch_future(probe2(log, url));
}
"#;
    let mut world = rut_native::dir_pkgs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"),
    )
    .expect("the async pair walks")
    .pkgs;
    let tree = std::path::Path::new(PKG_DIR);
    for d in ["ink_host", "http_host", "http"] {
        world.extend(
            rut_native::dir_pkgs(&tree.join(d))
                .unwrap_or_else(|e| panic!("mount {d}: {e}"))
                .pkgs,
        );
    }
    world.push(
        rut_driver::Mounted::source("app", src)
            // the app declares the heads it names (the nameability
            // law — mounted-but-undeclared is D5)
            .manifest(rut_driver::Manifest::lib(
                "app",
                &["http", "http_host", "futures", "async_host", "ink_host"],
            )),
    );
    let compiled = rut_driver::Walk {
        pkgs: world.clone(),
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
    .entrypoint("app")
    .compile()
    .unwrap();
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags
    );
    let prog = compiled.graph.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let ctx = rut_driver::host_pkg_ctx(&world);
    let mut hosts = rut_vm::interp::HostRegistry::new();
    let sink2 = sink.clone();
    hosts.install_host_pkg(
        &ctx,
        logger::pkg(move |m| sink2.borrow_mut().push(m.to_string())),
    );
    hosts.install_host_pkg(&ctx, async_host::pkg());
    // the http face's responses build through the strbuild pkg's
    // StringBuilder — its `strbuild_host` rows are declared here, so
    // the bodies bind alongside the async set
    hosts.install_host_pkg(&ctx, strbuild::pkg());
    let (http_pkg, fx) = pkg_with(|_m, _u, _h, _b| {
        Ok(FixtureReply::chunked(
            200,
            vec![b"ab".to_vec(), b"cd".to_vec()],
        ))
    });
    hosts.install_host_pkg(&ctx, http_pkg);
    hosts.verify_against(&ctx.flatten());
    let limits = rut_vm::interp::Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hosts(hosts)
        .build()
        .unwrap();
    vm.call::<_, ()>("boot", ("fixture://oneshot",)).unwrap();
    run_loop(&mut vm, &fx, 200);
    // the two launched futures interleave — the LAWS are
    // per-response, so compare as a set
    let mut got = sink.borrow().clone();
    got.sort();
    assert_eq!(
        got,
        vec![
            "body=4",             // probe: the drain took the whole body
            "first-next=2",       // probe2: the stream lane took chunk one
            "late-body=0",        // the late DRAIN degrades to empty
            "late-err=nil",       // degraded, not failed
            "late-next-nil=true", // the late mint is a dead reader
        ],
        "body xor stream: whichever lane took first wins, the second degrades"
    );
}

#[test]
fn the_builder_walks_every_verb_and_header() {
    let src = r#"
use http::{ HttpClient, ClientQueryMethod };
use futures::launch_future;
use ink_host::{ create_logger, logger_log };

async fn probe(log: opaque) -> nil {
    let client = HttpClient::new();
    let _ = await client.get("fixture://get").build().send();
    let _ = await client.post("fixture://post").body("p".encode()).build().send();
    let _ = await client.put("fixture://put").body("p".encode()).build().send();
    let _ = await client.patch("fixture://patch").body("p".encode()).build().send();
    let _ = await client.del("fixture://delete").build().send();
    let _ = await client.request()
        .method(ClientQueryMethod::Put)
        .url("fixture://manual")
        .header("Accept", "application/json")
        .header("X-Trace", "t-1")
        .body("m".encode())
        .build()
        .send();
    logger_log(log, 2, "all-sent");
}

entry fn boot() -> nil {
    let log = create_logger("t");
    launch_future(probe(log));
}
"#;
    let mut world = rut_native::dir_pkgs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"),
    )
    .expect("the async pair walks")
    .pkgs;
    let tree = std::path::Path::new(PKG_DIR);
    for d in ["ink_host", "http_host", "http"] {
        world.extend(
            rut_native::dir_pkgs(&tree.join(d))
                .unwrap_or_else(|e| panic!("mount {d}: {e}"))
                .pkgs,
        );
    }
    world.push(
        rut_driver::Mounted::source("app", src)
            // the app declares the heads it names (the nameability
            // law — mounted-but-undeclared is D5)
            .manifest(rut_driver::Manifest::lib(
                "app",
                &["http", "http_host", "futures", "async_host", "ink_host"],
            )),
    );
    let compiled = rut_driver::Walk {
        pkgs: world.clone(),
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
    .entrypoint("app")
    .compile()
    .unwrap();
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags
    );
    let prog = compiled.graph.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let ctx = rut_driver::host_pkg_ctx(&world);
    let mut hosts = rut_vm::interp::HostRegistry::new();
    let sink2 = sink.clone();
    hosts.install_host_pkg(
        &ctx,
        logger::pkg(move |m| sink2.borrow_mut().push(m.to_string())),
    );
    hosts.install_host_pkg(&ctx, async_host::pkg());
    // the http face's responses build through the strbuild pkg's
    // StringBuilder — its `strbuild_host` rows are declared here, so
    // the bodies install alongside the async set
    hosts.install_host_pkg(&ctx, strbuild::pkg());
    let seen = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    let (http_pkg, fx) = pkg_with(move |method, url, headers, body| {
        seen2.borrow_mut().push(format!(
            "{method} {url} headers=[{headers}] body={}",
            String::from_utf8_lossy(body)
        ));
        Ok(FixtureReply::ok(200, b""))
    });
    hosts.install_host_pkg(&ctx, http_pkg);
    hosts.verify_against(&ctx.flatten());
    let limits = rut_vm::interp::Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hosts(hosts)
        .build()
        .unwrap();
    vm.call::<_, ()>("boot", ()).unwrap();
    run_loop(&mut vm, &fx, 400);
    assert_eq!(*sink.borrow(), vec!["all-sent"]);
    assert_eq!(
        *seen.borrow(),
        vec![
            "GET fixture://get headers=[] body=",
            "POST fixture://post headers=[] body=p",
            "PUT fixture://put headers=[] body=p",
            "PATCH fixture://patch headers=[] body=p",
            "DELETE fixture://delete headers=[] body=",
            "PUT fixture://manual headers=[Accept: application/json\nX-Trace: t-1] body=m",
        ],
        "the five verbs are build sugars; headers accumulate Name: value lines"
    );
}

#[test]
fn the_decl_rows_join_both_lanes() {
    let world = rut_native::dir_pkgs(&std::path::Path::new(PKG_DIR).join("http_host"))
        .expect("mount http_host");
    // the pilot's tree: net/ (send + the readbacks) and stream/ (the
    // body lanes) — every row carries the mod path it lives under,
    // and the registration strings are path-qualified
    let m = world.pkg("http_host").expect("http_host mounted");
    assert_eq!(m.mods.keys().collect::<Vec<_>>(), vec!["net", "stream"]);
    let MountedBody::Host { host_funcs, .. } = &m.body else {
        panic!("a host body")
    };
    assert_eq!(
        host_funcs
            .iter()
            .map(|r| r.full_name("http_host"))
            .collect::<Vec<_>>(),
        vec![
            "http_host::net::http_send",
            "http_host::net::http_status",
            "http_host::net::http_err",
            "http_host::net::http_read_err",
            "http_host::net::http_resp_stream",
            "http_host::stream::http_body",
            "http_host::stream::http_stream_next",
        ],
        "net first, then stream, declaration order — the split surface"
    );
    let ctx = rut_driver::host_pkg_ctx(&world.pkgs);
    // the fixture lane installs the whole family over the virtual clock
    let mut hosts = rut_vm::interp::HostRegistry::new();
    let (http_pkg, _fx) = pkg_with(|_m, _u, _h, _b| Ok(FixtureReply::ok(200, b"")));
    hosts.install_host_pkg(&ctx, http_pkg);
    hosts.verify_against(&ctx.flatten()); // panics on drift
                                          // the reqwest lane installs the SAME rows — no network at install time
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, pkg());
    hosts.verify_against(&ctx.flatten());
}

#[test]
fn a_worker_thread_streams_through_the_reqwest_lane() {
    // the REAL lane over a local TCP server: send resolves, the
    // stream walks the chunks, the sticky read err stays clean —
    // the thread law end to end
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf); // the request head (small)
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabc")
            .unwrap();
        sock.write_all(b"def").unwrap(); // the rest after the headers
        sock.flush().unwrap();
    });
    let url = format!("http://{addr}/stream");
    // reuse the standard boot with the REAL lane bolted on
    let mut world = rut_native::dir_pkgs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"),
    )
    .expect("the async pair walks")
    .pkgs;
    let tree = std::path::Path::new(PKG_DIR);
    for d in ["ink_host", "http_host", "http"] {
        world.extend(
            rut_native::dir_pkgs(&tree.join(d))
                .unwrap_or_else(|e| panic!("mount {d}: {e}"))
                .pkgs,
        );
    }
    let stream_src = r#"
use http::HttpClient;
use futures::launch_future;
use ink_host::{ create_logger, logger_log };

async fn streamed(log: opaque, url: str) -> nil {
    let client = HttpClient::new();
    let resp = await client.get(url).build().send();
    logger_log(log, 2, f"status={resp.status()}");
    let s = resp.byte_stream();
    let total: i32 = 0;
    let failed = false;
    while (true) {
        let c = await s.next();
        if (c == nil) {
            let e = s.error();
            if (e != nil) { failed = true; }
            break;
        }
        let chunk: bytes = c;
        total += chunk.len();
    }
    logger_log(log, 2, f"total={total}");
    logger_log(log, 2, f"failed={failed}");
}

entry fn boot_stream(url: str) -> nil {
    let log = create_logger("t");
    launch_future(streamed(log, url));
}
"#;
    world.push(
        rut_driver::Mounted::source("app", stream_src)
            // the app declares the heads it names (the nameability
            // law — mounted-but-undeclared is D5)
            .manifest(rut_driver::Manifest::lib(
                "app",
                &["http", "http_host", "futures", "async_host", "ink_host"],
            )),
    );
    let compiled = rut_driver::Walk {
        pkgs: world.clone(),
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
    .entrypoint("app")
    .compile()
    .unwrap();
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags
    );
    let prog = compiled.graph.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let ctx = rut_driver::host_pkg_ctx(&world);
    let mut hosts = rut_vm::interp::HostRegistry::new();
    let sink2 = sink.clone();
    hosts.install_host_pkg(
        &ctx,
        logger::pkg(move |m| sink2.borrow_mut().push(m.to_string())),
    );
    hosts.install_host_pkg(&ctx, async_host::pkg());
    // the http face's responses build through the strbuild pkg's
    // StringBuilder — its `strbuild_host` rows are declared here, so
    // the bodies bind alongside the async set
    hosts.install_host_pkg(&ctx, strbuild::pkg());
    hosts.install_host_pkg(&ctx, pkg()); // the reqwest lane
    hosts.verify_against(&ctx.flatten());
    let limits = rut_vm::interp::Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hosts(hosts)
        .build()
        .unwrap();
    vm.call::<_, ()>("boot_stream", (url,)).unwrap();
    // wall-clock pump: the worker thread settles the completers
    for _ in 0..2000 {
        vm.run_ready().expect("run_ready");
        if vm.pending_tasks() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    server.join().expect("the server thread");
    assert_eq!(
        *sink.borrow(),
        vec!["status=200", "total=6", "failed=false"],
        "the send resolved at headers; the pump walked the whole body"
    );
}

#[test]
fn a_cancelled_stream_read_aborts_the_wire() {
    // THE WIRE-ABORT over a real socket: headers + a first chunk land,
    // the server then trickles; the second read parks, the probe
    // aborts it, and the row's `__cancel` arm marks the source. The
    // reqwest pump — blocked in its socket read — wakes on the next
    // chunk, sees the mark at its between-chunk check, and DROPS THE
    // RESPONSE: the connection closes (the server observes the EOF).
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf); // the request head (small)
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9999\r\n\r\nabc")
            .unwrap();
        sock.flush().unwrap();
        // trickle: one small chunk per beat; watch for the client
        // hanging up (the wire-abort). The first beats must outlive
        // the test's clock advance (the abort lands inside one).
        sock.set_nonblocking(true).expect("nonblocking");
        let mut closed = false;
        for i in 0..25 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            match sock.read(&mut buf) {
                Ok(0) => {
                    closed = true; // the client dropped the connection
                    break;
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {
                    closed = true; // reset: the client dropped
                    break;
                }
            }
            let chunk = format!("chunk{i}");
            if sock.write_all(chunk.as_bytes()).is_err() {
                closed = true; // broken pipe: the client dropped
                break;
            }
            let _ = sock.flush();
        }
        closed
    });
    let url = format!("http://{addr}/abort");
    let src = r#"
use http_host::net::{ http_send, http_read_err, http_resp_stream };
use http_host::stream::{ http_stream_next };
use futures::{ launch_future, sleep, LaunchedFutureHandle };
use ink_host::{ create_logger, logger_log };

async fn probe(log: opaque, url: str) -> nil {
    let r: opaque = await http_send(opaque(nil), "GET", url, "", "".encode());
    let s: opaque = http_resp_stream(r);
    let first = await http_stream_next(s);
    let n: i32 = -1;
    if (first != nil) {
        let v: bytes = first;
        n = v.len();
    }
    logger_log(log, 2, f"first={n}");
    let h = launch_future(http_stream_next(s));
    await sleep(1);
    let ok = h.abort();
    logger_log(log, 2, f"abort={ok}");
    // the cancel arm fires at the cancelled frame's next turn
    await sleep(1);
    let second = await http_stream_next(s);
    if (second == nil) {
        logger_log(log, 2, "next=nil");
    } else {
        logger_log(log, 2, "next=chunk");
    }
    let re = http_read_err(r);
    if (re == nil) {
        logger_log(log, 2, "read-err=nil");
    } else {
        let t: str = re;
        logger_log(log, 2, f"read-err={t}");
    }
}

entry fn boot_abort(url: str) -> nil {
    let log = create_logger("t");
    launch_future(probe(log, url));
}
"#;
    let mut world = rut_native::dir_pkgs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"),
    )
    .expect("the async pair walks")
    .pkgs;
    let tree = std::path::Path::new(PKG_DIR);
    for d in ["ink_host", "http_host", "http"] {
        world.extend(
            rut_native::dir_pkgs(&tree.join(d))
                .unwrap_or_else(|e| panic!("mount {d}: {e}"))
                .pkgs,
        );
    }
    world.push(
        rut_driver::Mounted::source("app", src).manifest(rut_driver::Manifest::lib(
            "app",
            &["http", "http_host", "futures", "async_host", "ink_host"],
        )),
    );
    let compiled = rut_driver::Walk {
        pkgs: world.clone(),
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
    .entrypoint("app")
    .compile()
    .unwrap();
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled.graph.diags
    );
    let prog = compiled.graph.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let sink = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let ctx = rut_driver::host_pkg_ctx(&world);
    let mut hosts = rut_vm::interp::HostRegistry::new();
    let sink2 = sink.clone();
    hosts.install_host_pkg(
        &ctx,
        logger::pkg(move |m| sink2.borrow_mut().push(m.to_string())),
    );
    hosts.install_host_pkg(&ctx, async_host::pkg());
    hosts.install_host_pkg(&ctx, strbuild::pkg());
    hosts.install_host_pkg(&ctx, pkg()); // the reqwest lane
    hosts.verify_against(&ctx.flatten());
    let limits = rut_vm::interp::Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hosts(hosts)
        .build()
        .unwrap();
    vm.call::<_, ()>("boot_abort", (url,)).unwrap();
    // the sleep(1) arming: the probe's wall-clock send resolves, the
    // first read takes the pre-landed chunk, the second read parks,
    // the abort sleep arms
    for _ in 0..500 {
        vm.run_ready().expect("run_ready");
        if vm.next_deadline().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(vm.next_deadline().is_some(), "the abort sleep armed");
    let due = vm.next_deadline().expect("armed");
    vm.set_now(due);
    let _ = vm.next_deadline();
    // the probes' tail sleeps + the arm's turn; the worker is
    // wall-clock — the server's trickle carries the wake to the pump
    for _ in 0..400 {
        vm.run_ready().expect("run_ready");
        if vm.next_deadline().is_some() {
            let due = vm.next_deadline().expect("armed");
            vm.set_now(due);
            let _ = vm.next_deadline();
        }
        if vm.pending_tasks() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let closed_by_server = server.join().expect("the server thread");
    assert!(
        closed_by_server,
        "the server observed the connection drop — the pump dropped the \
         response on the mark"
    );
    assert_eq!(
        *sink.borrow(),
        vec!["first=3", "abort=true", "next=nil", "read-err=cancelled"],
        "the aborted read marked the source; the next read degrades to nil \
         with the named error — never a trap"
    );
}
