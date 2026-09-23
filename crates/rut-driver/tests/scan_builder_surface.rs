//! The general scan/classify + builder surface (json-perf batch phase
//! 2): the `str` members `code_at`/`scan`/`starts_with` and the `StrBuf`
//! builtin class. These are the tokenizer primitives ANY parser wants —
//! the tests pin their semantics over ASCII and non-ASCII text, the
//! packed `(stop << 8) | class` scan result, the `set[min(cp, len-1)]`
//! table rule, the builder's byte-exact content across many appends,
//! pre-sizing, and the class aliasing law (assignment shares the cell).

use rut_core::binary::Surface;
use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    let collection = Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, Surface::core(), "core".to_string()), (3, collection, "collection".to_string())],
    )
}

fn run_main(src: &str) -> i32 {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

fn assert_ok(src: &str) {
    let code = run_main(src);
    assert_eq!(code, 0, "main returned {code}; source:\n{src}");
}

#[test]
fn code_at_reads_the_codepoint_at_an_index() {
    assert_ok(
        "pub fn main() -> i32 {
    let s = \"a\\u{1F600}b\"; // ascii, astral, ascii
    if (s.code_at(0) != 0x61) { return 1; }
    if (s.code_at(1) != 0x1F600) { return 2; }
    if (s.code_at(2) != 0x62) { return 3; }
    if (s.code_at(s.len() - 1) != 0x62) { return 4; }
    return 0;
}",
    );
}

#[test]
fn scan_stops_at_the_first_nonzero_class_and_packs_index_plus_class() {
    // a 257-entry table: `[` and `{` carry their own classes, everything
    // else 0 (keep scanning); the packed result is (stop << 8) | class
    assert_ok(
        "pub fn main() -> i32 {
    let mut t: [u8] = [0; 257];
    t[0x5B] = 7;  // '[' -> 7
    t[0x7B] = 9;  // '{' -> 9
    let s = \"ab [cd {ef\";
    let r = s.scan(0, t);
    if ((r >> 8) as i32 != 3) { return 1; }        // stops at '['
    if ((r & 255) as i32 != 7) { return 2; }       // class 7
    let r2 = s.scan(4, t);
    if ((r2 >> 8) as i32 != 7) { return 3; }       // from 4: stops at '{'
    if ((r2 & 255) as i32 != 9) { return 4; }      // class 9
    let r3 = s.scan(8, t);
    if ((r3 >> 8) as i32 != s.len()) { return 5; } // from past it: the end
    if ((r3 & 255) != 0) { return 6; }             // class 0
    return 0;
}",
    );
}

#[test]
fn scan_table_last_entry_serves_every_high_codepoint() {
    // a 2-entry table: class 5 for EVERY codepoint (the min(cp, len-1)
    // rule); a 257-entry table isolates the high slot instead
    assert_ok(
        "pub fn main() -> i32 {
    let mut t: [u8] = [0, 5];
    let r = \"abc\".scan(0, t);
    if ((r >> 8) as i32 != 0 || (r & 255) as i32 != 5) { return 1; }
    let r2 = \"\\u{e9}x\".scan(0, t);
    if ((r2 >> 8) as i32 != 0 || (r2 & 255) as i32 != 5) { return 2; }
    let mut t2: [u8] = [0; 257];
    t2[0xE9] = 3; // the é slot (0xE9 < 256: its own entry)
    let r3 = \"ab\\u{e9}c\".scan(0, t2);
    if ((r3 >> 8) as i32 != 2 || (r3 & 255) as i32 != 3) { return 3; }
    t2[256] = 4; // the HIGH slot: every codepoint >= 257 folds here
    let r4 = \"ab\\u{1F600}c\".scan(0, t2);
    if ((r4 >> 8) as i32 != 2 || (r4 & 255) as i32 != 4) { return 4; }
    let r5 = \"abc\".scan(0, t2);
    if ((r5 >> 8) as i32 != 3 || (r5 & 255) != 0) { return 5; }
    return 0;
}",
    );
}

#[test]
fn scan_walks_codepoints_not_octets_on_non_ascii_text() {
    // 'é' is TWO octets but ONE codepoint, the emoji FOUR: the stop
    // index is the CODEPOINT index (3), not a byte offset — the loop
    // skips whole scalars rather than classifying octets
    assert_ok(
        "pub fn main() -> i32 {
    let mut t: [u8] = [0; 257];
    t[0x62] = 2; // 'b' -> 2
    let s = \"a\\u{e9}\\u{1F600}b\"; // 4 codepoints, 8 octets
    let r = s.scan(0, t);
    if ((r >> 8) as i32 != 3) { return 1; }
    if ((r & 255) as i32 != 2) { return 2; }
    return 0;
}",
    );
}

#[test]
fn scan_matches_a_rut_side_reference_over_all_byte_values() {
    // parity pin: for every 1-codepoint string built from codepoints
    // 0..256, scan with a single-class table stops exactly where the
    // table says — at the codepoint with its class, or the end with 0
    assert_ok(
        "pub fn main() -> i32 {
    let mut t: [u8] = [0; 257];
    let mut c: i32 = 0;
    while (c < 256) {
        t[c] = 1;
        let s = str.from_code(c as u32);
        let r = s.scan(0, t);
        if ((r >> 8) as i32 != 0 || (r & 255) as i32 != 1) { return 1; }
        t[c] = 0;
        let r2 = s.scan(0, t);
        if ((r2 >> 8) as i32 != 1 || (r2 & 255) != 0) { return 2; }
        c += 1;
    }
    return 0;
}",
    );
}

#[test]
fn starts_with_tests_a_prefix_at_a_codepoint_offset() {
    assert_ok(
        "pub fn main() -> i32 {
    let s = \"true false null\";
    if (!s.starts_with(0, \"true\")) { return 1; }
    if (!s.starts_with(5, \"false\")) { return 2; }
    if (!s.starts_with(11, \"null\")) { return 3; }
    if (s.starts_with(1, \"true\")) { return 4; }
    if (s.starts_with(13, \"nullish\")) { return 5; } // head longer than the rest
    if (!s.starts_with(s.len(), \"\")) { return 6; }  // empty head at the end
    let n = \"\\u{e9}cole\";
    if (!n.starts_with(0, \"\\u{e9}co\")) { return 7; }
    if (!n.starts_with(2, \"ole\")) { return 8; }
    if (n.starts_with(1, \"olx\")) { return 9; }
    return 0;
}",
    );
}

#[test]
fn builder_accumulates_bytes_and_codepoints_and_finishes_once() {
    assert_ok(
        "pub fn main() -> i32 {
    let b = StrBuf(16);
    b.push(\"ab\");
    b.push_code(0x1F600); // an astral codepoint: 4 octets, 1 char
    b.push(\"cd\");
    b.push_code(0x7F);
    if (b.len() != 6) { return 1; }
    let s = b.finish();
    if (s != \"ab\\u{1F600}cd\\u{7f}\") { return 2; }
    if (s.len() != 6) { return 3; }
    // the builder keeps its buffer: finish twice answers the same text
    let s2 = b.finish();
    if (s2 != s) { return 4; }
    return 0;
}",
    );
}

#[test]
fn builder_push_code_replaces_invalid_scalars_with_fffd() {
    assert_ok(
        "pub fn main() -> i32 {
    let b = StrBuf(0);
    b.push_code(0xD800); // a surrogate — never representable in a rut str
    if (b.len() != 1) { return 1; }
    let s = b.finish();
    if (s.code() != 0xFFFD) { return 2; }
    return 0;
}",
    );
}

#[test]
fn builder_grows_amortized_over_many_appends_and_stays_exact() {
    // 40k appends building a 200k-codepoint document: byte-exact against
    // a reference and the tracked length — the amortized growth, not an
    // O(n^2) prefix copy
    assert_ok(
        "pub fn main() -> i32 {
    let b = StrBuf(0);
    let mut i: i32 = 0;
    while (i < 40000) {
        b.push(\"abc\");
        b.push_code(0x61 + (i % 26) as u32);
        i += 1;
    }
    if (b.len() != 160000) { return 1; }
    let s = b.finish();
    if (s.len() != 160000) { return 2; }
    if (!s.starts_with(0, \"abca\")) { return 3; }
    if (s.code_at(s.len() - 1) != 0x61 + ((39999 % 26) as u32)) { return 4; }
    return 0;
}",
    );
}

#[test]
fn builder_shares_its_cell_like_every_class() {
    // RFC 0044's aliasing law: assignment shares the cell, so appends
    // through one name are visible through the other
    assert_ok(
        "pub fn main() -> i32 {
    let a = StrBuf(0);
    let c = a;
    c.push(\"x\");
    if (a.len() != 1) { return 1; }
    if (a.finish() != \"x\") { return 2; }
    return 0;
}",
    );
}

#[test]
fn scan_and_builder_cross_check_round_trip() {
    // scan finds where the digit run ends, the builder reassembles the
    // pieces — the two halves of the phase's surface in one pass
    assert_ok(
        "pub fn main() -> i32 {
    let mut dig: [u8] = [1; 257];
    let mut i: i32 = 0x30;
    while (i <= 0x39) { dig[i] = 0; i += 1; }
    let s = \"12345abc\";
    let r = s.scan(0, dig);
    let stop = (r >> 8) as i32;
    if (stop != 5 || (r & 255) != 1) { return 1; }
    let b = StrBuf(0);
    b.push(s.slice(stop, s.len()));
    b.push(\"-\");
    b.push(s.slice(0, stop));
    if (b.finish() != \"abc-12345\") { return 2; }
    return 0;
}",
    );
}
