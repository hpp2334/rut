//! Host-constructed `opaque` boxes (RFC 0023/0026): the embedder news any
//! `'static` Rust value behind an `opaque`, borrows it back typed, and rut
//! sees only the box. RFC 0014 semantics must hold on the rut side —
//! `o is opaque` is `true`, `opaque.downcast<T>` is `None` for every `T` (never a
//! trap), `own` shares identity — and the payload's `Drop` runs when the
//! box's rc hits 0 (RFC 0016 §3).
//!
//! The drop-timing flow drives the box through a wrapper-class record as
//! well as direct host calls — the record's field retain releases with the
//! record at rc-0, so the payload's `Drop` runs with the last handle even
//! with the wrapper in the mix (RFC 0016 §3's recursive field walk).

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use rut_vm::{Opaque, OpaqueRef, Trap};
use rut_vm::interp::Vm;

const SRC: &str = r#"
use boxes::{ store_new, store_set, store_get, store_size };

// the wrapper class (the Logger pattern, RFC 0028): one opaque field,
// one host fn call per method — the Rust payload never leaks into rut
class Store {
    h: opaque;
}
impl Store {
    fn adopt(h: opaque) -> Self { return Self { h: h }; }
    fn set(mut self, k: str, v: i64) -> nil { store_set(self.h, k, v); }
    fn get(self, k: str) -> i64 { return store_get(self.h, k); }
}

entry fn make() -> opaque { return store_new(); }
entry fn put(c: opaque, k: str, v: i64) -> nil { store_set(c, k, v); }
entry fn get(c: opaque, k: str) -> i64 { return store_get(c, k); }
entry fn size(c: opaque) -> i64 { return store_size(c); }
entry fn wrap_put(c: opaque, k: str, v: i64) -> nil {
    let s = Store.adopt(c);
    s.set(k, v);
}
entry fn wrap_get(c: opaque, k: str) -> i64 {
    return Store.adopt(c).get(k);
}
entry fn erase_laws(c: opaque) -> bool {
    // RFC 0014 on a host payload box: it is an opaque and nothing more
    // specific — no rut type recovers from it, the recovery is `nil`,
    // never a trap
    let i64_hit = opaque.downcast<i64>(c) != nil;
    let str_hit = opaque.downcast<str>(c) != nil;
    return c is opaque && !i64_hit && !str_hit;
}
entry fn share(c: opaque) -> opaque {
    // a host box IS a handle — returning it shares identity (v1.1)
    return c;
}
"#;

struct Store {
    map: HashMap<String, i64>,
    dropped: Rc<Cell<bool>>,
}

impl Drop for Store {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}

struct Widget {
    _x: i64,
}

type ExpectedHostFns = std::collections::BTreeMap<
    String,
    (Vec<rut_core::types::TypeId>, rut_core::types::TypeId),
>;

fn session(dropped: &Rc<Cell<bool>>) -> rut_vm::interp::Vm {
    let mut session = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut session);
    // the sources use `pouch` — a third-party pkg, mounted from the tree
    rut_driver::mount_dir(
        &mut session,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/pouch"),
    )
    .expect("mount pouch");
    // `boxes` — this test's own host pkg, declared in tests/data/boxes
    rut_driver::mount_dir(
        &mut session,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/boxes"),
    )
    .expect("mount boxes");
    let expected = session.expected_host_fns();
    session
        .register_module(
            "app_boxes",
            rut_driver::Module { spec: "app_boxes".into(), source: Some(SRC.into()), ..Default::default() },
        )
        .unwrap();
    let g = rut_driver::compile_graph(&session, "app_boxes");
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = g.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // bindings BEFORE the Vm (RFC 0025): install + contract + boot
    let mut hosts = rut_vm::interp::HostRegistry::new();
    install(&mut hosts, dropped);
    hosts.verify_against(&expected); // tests/data/boxes/boxes.d.rut ↔ the bodies
    rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts).unwrap()
}

/// Bind the `boxes` bodies over a real `HashMap` payload. Typed per
/// tests/data/boxes/boxes.d.rut; the contract verifies in `session`.
fn install(hosts: &mut rut_vm::interp::HostRegistry, dropped: &Rc<Cell<bool>>) {
    let dropped = dropped.clone();
    rut_vm::register!(hosts, "boxes::store_new", () -> OpaqueRef, move |vm: &mut Vm| {
        let b = Opaque::alloc(vm, Store { map: HashMap::new(), dropped: dropped.clone() })?;
        Ok(b.handle().clone())
    });
    rut_vm::register!(
        hosts,
        "boxes::store_set",
        (Opaque<Store>, &str, i64) -> (),
        |vm: &mut Vm, b: Opaque<Store>, k: &str, v: i64| {
            b.with_mut(vm, |_vm, s| {
                s.map.insert(k.to_string(), v);
            })
        },
    );
    rut_vm::register!(
        hosts,
        "boxes::store_get",
        (Opaque<Store>, &str) -> i64,
        |_vm: &mut Vm, b: Opaque<Store>, k: &str| -> Result<i64, Trap> {
            Ok(b.with(|s| s.map.get(k).copied())?.unwrap_or(-1))
        },
    );
    rut_vm::register!(
        hosts,
        "boxes::store_size",
        (Opaque<Store>,) -> i64,
        |_vm: &mut Vm, b: Opaque<Store>| b.with(|s| s.map.len() as i64)
    );
}

#[test]
fn host_boxes_hold_any_rust_type() {
    let dropped = Rc::new(Cell::new(false));
    let mut vm = session(&dropped);
    
    // news from the host, hold the handle across calls — the data lives
    // in the VM heap as long as either side holds a reference
    let held: OpaqueRef = vm.call("make", ()).unwrap();

    // drive it from rut, both through the wrapper class and direct — the
    // wrapper's records release the box's field retain when they die, so
    // this is the flow that needs the recursive field release to hold
    vm.call::<_, ()>("wrap_put", (held.clone(), "via-wrapper", 1i64)).unwrap();
    assert_eq!(vm.call::<_, i64>("wrap_get", (held.clone(), "via-wrapper")).unwrap(), 1);
    vm.call::<_, ()>("put", (held.clone(), "key-3", 41i64)).unwrap();
    vm.call::<_, ()>("put", (held.clone(), "key-7", 7i64)).unwrap();
    assert_eq!(vm.call::<_, i64>("get", (held.clone(), "key-3")).unwrap(), 41);
    assert_eq!(vm.call::<_, i64>("size", (held.clone(),)).unwrap(), 3);
    assert_eq!(vm.call::<_, i64>("get", (held.clone(), "missing")).unwrap(), -1);

    // RFC 0014 erasure laws on the rut side
    let erased: bool = vm.call("erase_laws", (held.clone(),)).unwrap();
    assert!(erased, "erase_laws");

    // own shares identity: writes through the copy are visible through
    // the original, and the handles are `==`
    let shared: OpaqueRef = vm.call("share", (held.clone(),)).unwrap();
    vm.call::<_, ()>("put", (shared.clone(), "via-own", 9i64)).unwrap();
    assert_eq!(vm.call::<_, i64>("get", (held.clone(), "via-own")).unwrap(), 9);
    assert_eq!(shared, held);

    // a wrong-type borrow is a checked error naming both sides
    let err = Opaque::<Widget>::from_handle(&held).err().expect("wrong type must be rejected");
    assert!(err.msg.contains("Store") && err.msg.contains("Widget"), "{}", err.msg);

    // a non-handle is rejected too
    let err = Opaque::<Widget>::from_handle(&held).err().expect("non-handle must be rejected");
    assert!(err.msg.contains("host box holds") || err.msg.contains("opaque"), "{}", err.msg);

    // the borrow guard (RFC 0023 §2): a nested exclusive borrow is a
    // checked trap, and a shared borrow is excluded while `&mut` is out
    let b = Opaque::<Store>::from_handle(&held).unwrap();
    b.with_mut(&mut vm, |vm, _| {
        assert!(b.with(|_| {}).is_err(), "shared borrow must be excluded under &mut");
        let again = b.with_mut(vm, |_vm, _| {});
        assert!(again.unwrap_err().msg.contains("borrowed by an outer host call"));
    })
    .unwrap();
    b.with(|_| {}).expect("the guard clears when the closure returns");

    // rc-0 (RFC 0016 §3): the wrapper records are gone (their field
    // retains released with them), so the box's only references are the
    // three handles — the payload's Drop runs with the last one
    drop(held);
    drop(shared);
    assert!(!dropped.get(), "b still holds");
    drop(b);
    assert!(dropped.get(), "payload Drop must run deterministically at rc-0");
    drop(vm);
    assert!(dropped.get());
}

#[test]
fn record_fields_release_at_rc0() {
    // the direct proof of the recursive field release: pure rut, class
    // instances with `str` / `opaque` / `Vec<str>` fields created and
    // dropped in a loop — before the fix every record leaked its fields'
    // references and the heap climbed by ~n cells; now it returns to
    // baseline (plus the run's immortal singletons).
    let src = r#"
use pouch::{ Vec };

class Node {
    name: str;
    tag: opaque;
    tags: Vec<str>;
}
impl Node {
    fn new(name: str) -> Self { return Self { name: name, tag: opaque(0), tags: Vec.new() }; }
}

entry fn churn(n: i64) -> nil {
    for (let i = 0i64; i < n; i += 1) {
        let node = Node.new(f"n{i}");
        node.tags.push(f"t{i}");
    }
}
"#;
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut s);
    rut_driver::mount_dir(
        &mut s,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/pouch"),
    )
    .expect("mount pouch");
    let out = rut_driver::compile_module_in(&mut s, src, rut_parser::Mode::Impl, "churn");
    assert!(
        out.diags.is_empty(),
        "{}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(10_000_000),
        heap_limit_bytes: Some(8 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut hosts0 = rut_vm::interp::HostRegistry::new();
    rut_std::logger::install_std_log(&mut hosts0, |_msg| {});
    rut_std::math::install_std_math(&mut hosts0);
    let mut vm =
        rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts0).unwrap();

    vm.call::<_, ()>("churn", (1i64,)).unwrap();
    let base = vm.heap_usage();
    vm.call::<_, ()>("churn", (2000i64,)).unwrap();
    let after = vm.heap_usage();
    assert!(
        after <= base + 4096,
        "records must release their fields at rc-0: baseline {base}, after churn(2000): {after}"
    );
}

