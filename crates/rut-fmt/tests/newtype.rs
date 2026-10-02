//! The newtype decl's formatter round trip: `class Name(Wrapped);`
//! prints back in its positional spelling — never as the desugared
//! braced body — reparses clean, and is idempotent.

use rut_parser::Mode;
use rut_fmt::format;

fn fmt(src: &str) -> String {
    let out = format(src, Mode::Impl, &rut_fmt::Style::default()).expect("formats");
    out
}

#[test]
fn positional_decl_prints_back_positional() {
    let out = fmt("class JsonI64(i64);\n");
    assert!(
        out.contains("class JsonI64(i64);"),
        "the positional spelling survives: {out:?}"
    );
    assert!(!out.contains("{ inner"), "never the desugared body: {out:?}");
}

#[test]
fn generic_and_pub_heads_print_positional() {
    for src in [
        "pub class TheirJson(TheirType);\n",
        "class Tail<T>([T]);\n",
        "class HashWrap<K requires Hashable>(K);\n",
    ] {
        let out = fmt(src);
        let want = src.trim_end_matches('\n');
        assert!(
            out.contains(want),
            "{src}: the spelling survives the printer: {out:?}"
        );
        // reparse + idempotence
        let again = format(&out, Mode::Impl, &rut_fmt::Style::default()).expect("reformats");
        assert_eq!(out, again, "{src}: idempotent");
    }
}
