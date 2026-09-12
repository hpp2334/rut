//! Module loading: compile a library, import one of its functions, link
//! (RFC 0029 surface / RFC 0035 §1).

use rut_parser::Mode;
use rut_driver::{Module, Session};

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

#[test]
fn imports_and_links_a_type() {
    // dependency "geo" scope 1 exports a record + a fn returning it
    let dep = rut_driver::compile_program(
        "dataclass Point { x: i32; y: i32; }\n\
         pub fn origin() -> Point { return Point { x: 0, y: 0 }; }\n\
         fn main() -> i32 { return 0; }\n",
        Mode::Impl,
        "geo",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");
    let surface = dep.surface.clone();
    assert!(
        surface.type_exports.iter().any(|t| t.name == "Point"),
        "surface exports Point"
    );

    // root "app" scope 2 constructs the imported type and reads its fields
    let root = rut_driver::compile_program(
        "import { Point, origin } from \"geo\";\n\
         fn mk() -> Point { return Point { x: 1, y: 2 }; }\n\
         fn main() -> i32 {\n\
             let p: Point = mk();\n\
             let q: Point = origin();\n\
             return p.x + q.y;\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface)],
    );
    assert!(root.diags.is_empty(), "{:?}", root.diags);
    let app_ir = root.ir_dump.clone();
    let root = root.program.expect("root program");

    let linked = rut_core::link::link(vec![dep, root]).expect("link");
    // the imported type made it into the global table once
    let point_ids: Vec<u32> = (0..linked.types.types.len() as u32)
        .filter(|&i| linked.types.name(i) == "Point")
        .collect();
    assert_eq!(point_ids.len(), 1, "Point appears exactly once");
    let point = point_ids[0];
    // app's own funcs are appended after geo's (main, origin)
    let app: Vec<&rut_core::binary::FuncCode> = linked.funcs.iter().skip(2).collect();
    let made = app.iter().flat_map(|f| &f.code).any(|op| matches!(
        op,
        rut_core::ops::Op::MakeRecord { ty, .. } if *ty == point
    ));
    let read = app.iter().flat_map(|f| &f.code).any(|op| matches!(op, rut_core::ops::Op::GetF { .. }));
    assert!(made, "app constructs the imported Point\n{app_ir}");
    assert!(read, "app reads an imported field\n{app_ir}");
}

#[test]
fn graph_compiles_and_links_imports_in_order() {
    let mut s = Session::new();
    s.register_module(
        "std:math",
        Module {
            source: Some(
                "pub fn seven() -> i32 { return 7; }\n\
                 fn main() -> i32 { return seven(); }\n"
                    .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    s.register_module(
        "app:main",
        Module {
            source: Some(
                "import { seven } from \"std:math\";\n\
                 fn main() -> i32 { return seven(); }\n"
                    .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();

    let out = rut_driver::compile_graph(&s, "app:main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("linked program");
    // std:math: main(0), seven(1); app:main: main(2)
    assert_eq!(p.funcs.len(), 3);
    let call = p.funcs[2]
        .code
        .iter()
        .find_map(|op| match op {
            rut_core::ops::Op::Call { func, .. } => Some(*func),
            _ => None,
        })
        .expect("app calls seven");
    assert_eq!(call, 1);
    let bytes = rut_core::binary::encode(&p);
    assert!(rut_core::binary::decode(&bytes).is_ok());
}

#[test]
fn graph_threads_a_type_through_a_chain() {
    let mut s = Session::new();
    s.register_module(
        "geo:base",
        Module {
            source: Some(
                "dataclass Point { x: i32; y: i32; }\n\
                 pub fn origin() -> Point { return Point { x: 0, y: 0 }; }\n\
                 fn main() -> i32 { return 0; }\n"
                    .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    s.register_module(
        "geo:mid",
        Module {
            source: Some(
                "import { Point, origin } from \"geo:base\";\n\
                 pub fn shifted() -> Point { return origin(); }\n\
                 fn main() -> i32 { return 0; }\n"
                    .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    s.register_module(
        "app:main",
        Module {
            source: Some(
                "import { Point } from \"geo:base\";\n\
                 import { shifted } from \"geo:mid\";\n\
                 fn main() -> i32 { let p: Point = shifted(); return p.x; }\n"
                    .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();

    let out = rut_driver::compile_graph(&s, "app:main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("linked program");
    let points = (0..p.types.types.len() as u32)
        .filter(|&i| p.types.name(i) == "Point")
        .count();
    assert_eq!(points, 1, "Point is shared across the chain, not duplicated");
}

#[test]
fn std_collection_barrel_expands_and_compiles() {
    // entry.rut inlines ./vec.rut + ./hash.rut into one module unit
    let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../rut/std-collection/entry.rut");
    let merged = rut_driver::expand_module_source(&entry).expect("expand");
    assert!(merged.contains("class Vec<T>"), "vec.rut inlined");
    let src = format!(
        "{merged}\nfn main() -> i32 {{\n\
             let mut v: Vec<i32> = Vec.new();\n\
             v.push(1);\n\
             v.push(2);\n\
             return v.len();\n\
         }}\n"
    );
    let out = rut_driver::compile_program(&src, Mode::Impl, "std:collection", 1, &[]);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    assert_eq!(p.types.types.iter().filter(|t| t.name == "Vec<i32>").count(), 1);
}

#[test]
fn loads_a_directory_graph() {
    let base = std::env::temp_dir().join(format!("rut-load-test-{}", std::process::id()));
    let app = base.join("app");
    let lib = base.join("lib");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(
        app.join("rut.toml"),
        "name = \"app:main\"\nentry.lib = \"./entry.rut\"\n[deps]\n\"lib:math\" = { path = \"../lib\" }\n",
    )
    .unwrap();
    std::fs::write(
        app.join("entry.rut"),
        "import { seven } from \"lib:math\";\nfn main() -> i32 { return seven(); }\n",
    )
    .unwrap();
    std::fs::write(
        lib.join("rut.toml"),
        "name = \"lib:math\"\nentry.lib = \"./lib.rut\"\n",
    )
    .unwrap();
    std::fs::write(lib.join("lib.rut"), "pub fn seven() -> i32 { return 7; }\n").unwrap();

    let out = rut_driver::compile_dir(&app).expect("compile_dir");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("linked program");
    // lib:math::seven called from app:main
    let call = p
        .funcs
        .iter()
        .flat_map(|f| &f.code)
        .find_map(|op| match op {
            rut_core::ops::Op::Call { func, .. } => Some(*func),
            _ => None,
        });
    assert!(call.is_some(), "cross-module call present");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn graph_reports_a_missing_dependency() {
    let mut s = Session::new();
    s.register_module(
        "app:main",
        Module {
            source: Some(
                "import { nope } from \"std:missing\";\n\
                 fn main() -> i32 { return 0; }\n"
                    .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app:main");
    assert!(out.program.is_none());
    assert!(
        out.diags.iter().any(|d| d.msg.contains("package `std` is not mounted")),
        "{:?}",
        out.diags
    );
}
