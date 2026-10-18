//! Array-literal reference ownership: an `[ref, ..]` literal's elements
//! must stay alive after the creating frame exits (the temps' references
//! end there; the array's own references — released by the release walk's
//! element collection — must be real). Regression: `ArrLit` used to copy
//! the element slots into the raw block WITHOUT retaining them, so any
//! element read after the literal's creating frame returned resolved a
/// recycled arena slot.

/// Compile, flatten, verify, and run a single-module `main` returning i32.
use rut_parser::Mode;

fn run_main(src: &str) -> i32 {
    let out = rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string())],
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(prog))
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

/// The literal's element temps die at `make`'s return; reading `a[0]`
/// afterwards must still see the minted value (pre-fix: the element cell
/// was freed under the array and the slot reused by the churn between).
#[test]
fn arr_lit_element_outlives_creating_frame() {
    let v = run_main(
        r#"
        class Box { v: i64; }
        fn make() -> [Box] {
          return [Box { v: 11i64 }, Box { v: 22i64 }];
        }
        entry fn main() -> i32 {
          let a = make();
          // churn: force arena recycling so a stale slot would be reused
          let j1 = Box { v: 100i64 };
          let j2 = Box { v: 101i64 };
          let j3 = Box { v: 102i64 };
          let j4 = Box { v: 103i64 };
          return ((a[0].v + a[1].v) + (j1.v + j2.v + j3.v + j4.v) - 406i64) as i32;
        }
        "#,
    );
    assert_eq!(v, 33);
}

/// The closure flavor: the array is captured, the element read happens
/// inside the closure invoked after `make`'s frame is long gone — the
/// mutation-closure shape from the tur investigation.
#[test]
fn arr_lit_element_in_late_closure() {
    let v = run_main(
        r#"
        class Box { v: i64; }
        fn make() -> [Box] {
          return [Box { v: 5 }, Box { v: 6 }];
        }
        entry fn main() -> i32 {
          let a = make();
          let f = fn (x: i64) -> i64 {
            return a[x as i32].v;
          };
          let mut churn = Box { v: 0 };
          churn = Box { v: 1 };
          return (f(0) + f(1)) as i32;
        }
        "#,
    );
    assert_eq!(v, 11);
}

/// Interface dispatch on an element-loaded value after the creating frame
/// exits — the `no impl for interface slot N on <recycled type>` shape.
#[test]
fn arr_lit_element_interface_dispatch_after_exit() {
    let v = run_main(
        r#"
        interface Get { fn get(self) -> i64; }
        class Box { v: i64; }
        impl Box { pub fn get(self) -> i64 { return self.v; } }
        fn make() -> [Box] {
          return [Box { v: 40 }, Box { v: 2 }];
        }
        entry fn main() -> i32 {
          let a = make();
          let f = fn () -> i64 {
            let g: Get = a[0];
            return g.get();
          };
          return f() as i32;
        }
        "#,
    );
    assert_eq!(v, 40);
}

/// Str elements are ref cells too — the same law on the str lane.
#[test]
fn arr_lit_str_element_after_exit() {
    let v = run_main(
        r#"
        fn make() -> [str] {
          return ["alpha", "beta"];
        }
        entry fn main() -> i32 {
          let a = make();
          let f = fn () -> i64 {
            return (a[0].len() + a[1].len()) as i64;
          };
          return f() as i32;
        }
        "#,
    );
    assert_eq!(v, 9);
}

/// The erasure box's seal type must follow the GENERIC argument: a u64
/// boxed inside a generic body must unbox as u64 (not the erased i64
/// default) — the reactive `source<u64>` stamp lane depends on it.
#[test]
fn generic_box_seal_follows_type_argument() {
    let v = run_main(
        r#"
        fn wrap<T>(v: T) -> opaque {
          return opaque(v);
        }
        entry fn main() -> i32 {
          let b = wrap<u64>(7u64);
          let x = unopaque<u64>(b);
          let c = wrap<i64>(9i64);
          let y = unopaque<i64>(c);
          return (x as i64 + y) as i32;
        }
        "#,
    );
    assert_eq!(v, 16);
}
