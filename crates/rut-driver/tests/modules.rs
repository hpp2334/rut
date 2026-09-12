//! Module loading: compile a library, import one of its functions, link
//! (RFC 0029 surface / RFC 0035 §1).

use rut_parser::Mode;

#[test]
fn imports_and_links_a_function() {
    // dependency "math" under scope 1 (main keeps `add` monomorphized)
    let dep = rut_driver::compile_program(
        "pub fn add(a: i32, b: i32) -> i32 { return a + b; }\n\
         pub fn main() -> i32 { return add(1, 2); }\n",
        Mode::Impl,
        "math",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");
    let surface = dep.surface.clone();
    assert!(surface.funcs.iter().any(|f| f.name == "add"), "surface has add");

    // root "app" under scope 2 imports `add`
    let root = rut_driver::compile_program(
        "import { add } from \"math\";\nfn main() -> i32 { return add(2, 3); }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface)],
    );
    assert!(root.diags.is_empty(), "{:?}", root.diags);
    let root = root.program.expect("root program");

    let linked = rut_core::link::link(vec![dep, root]).expect("link");
    // math: main(0), add(1); app: main(2)
    assert_eq!(linked.funcs.len(), 3);
    let call = linked.funcs[2]
        .code
        .iter()
        .find_map(|op| match op {
            rut_core::ops::Op::Call { func, .. } => Some(*func),
            _ => None,
        })
        .expect("app calls add");
    assert_eq!(call, 1, "Call should target math::add");

    // binary round-trips
    let bytes = rut_core::binary::encode(&linked);
    let back = rut_core::binary::decode(&bytes).expect("decode");
    assert_eq!(back.funcs.len(), 3);
}

#[test]
fn imports_without_a_binding_still_error() {
    // `compile_module` has no imports: the old diagnostic stands
    let out = rut_driver::compile_module(
        "import { add } from \"math\";\nfn main() -> i32 { return add(1, 2); }\n",
        Mode::Impl,
        "app",
    );
    assert!(out.diags.iter().any(|d| d.msg.contains("module loading is not available")));
}
