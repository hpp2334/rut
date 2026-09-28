# 02 — Digest

The byte-level example, in the shape of
[00 — Todolist](00-todolist.md) and [01 — Sort](01-sort.md) — with a
twist: this time the embedder is also the **oracle**. `digest.rut` is
a byte-level library written in pure rut:

- **encodings** — hex (encode/decode, case-insensitive) and base64
  (standard + URL-safe alphabets, padding, invalid-input rejection)
- **crypto digests** — MD5, SHA-1, SHA-256, SHA-512 behind one
  `when`-on-string dispatcher
- **hashmap hash keys** — CRC-32, FNV-1a 32/64, djb2, sdbm
- **JSON** — decode to an `opaque` tree, encode back, round-trip,
  with numbers stored as verbatim lexemes so round-trips are exact

Nothing in the rut file trusts itself: the Rust host cross-checks
every algorithm against independent crates (the RustCrypto hash
family, `base64`, `crc32fast`, `serde_json`) on canonical test
vectors *and* on deterministic pseudo-random inputs at every
padding-edge length, plus a 64 KiB stress blob. `digest.rut` knows
nothing about the crates; only the host compares.

## Run it

```sh
cargo run -p digests       # from the repo root
cargo test -p digests      # the oracle, asserted
```

The run prints one verified row per check — abridged:

```text
hex([114, 117, 116, 33]) = 72757421
b64([102, 111, 111, 98, 97, 114], url=false) = Zm9vYmFy  OK
digests, rut vs crates:
  md5     empty         fuel    8419  d41d8cd98f00b204e9800998ecf8427e  OK
  sha256  "abc"         fuel   18866  ba7816bf...f20015ad  OK
  md5     1 KiB lcg(42) fuel  113117  d6c1961991b0106647e36ca4ba12d345  OK
sample_doc -> {"name":"rut","version":0.2,"tags":["tiny","fast","verified"],...}
serde_json parses it: OK
hash keys over that JSON text:
  crc32   eccc5960  OK
  fnv1a64 8ba697141da02e83  OK
hex_dec("zz")   = "hex: invalid character at index 0"
json_dec("{,}") = "json: expected a key string at index 1"
digest("md4")   = "unknown algorithm: md4"
every row agrees: OK
fuel used: 1014105 of Some(50000000)
```

## Code tour

### The integer surface is enough for real algorithms

`digest.rut` runs on hex literals, `u32`/`u64`, the wrapping family,
and signedness-correct shifts. CRC-32 is the compact showcase —
table-free, bitwise, and exactly the textbook loop:

```rut
entry fn crc32(data: bytes) -> u32 {
    let mut crc: u32 = 0xFFFFFFFFu32;
    for (let b of data) {
        crc = crc ^ b as u32;
        for (let k = 0; k < 8; k += 1) {
            if ((crc & 1) == 1) {
                crc = (crc >> 1) ^ 0xEDB88320u32;
            } else {
                crc = crc >> 1;
            }
        }
    }
    return crc ^ 0xFFFFFFFFu32;
}
```

SHA-512 is the demanding one: its 64-bit rotations need `>>` to be a
*logical* shift on `u64` and `wrapping_shl` to truncate to the operand
width. The FNV/djb2/sdbm trio, by contrast, deliberately never shifts
a 64-bit word — `wrapping_mul`, `wrapping_add`, and `^` carry them,
showing how far the primitive surface alone goes.

### `bytes` crosses directly; `opaque` appears exactly once

Byte payloads need no wrapper — `bytes` is in the crossing set, so
`entry fn hex_enc(data: bytes) -> str` takes a host byte slice
head-on ([primitive types](../reference/primitive-types.md)). The one
erasure in the file is the recursive JSON tree: rut has no recursive
dataclass, so the tree is a tagged union by hand with children boxed
in `opaque` to break the recursion
([opaque — erasure and downcast](../reference/opaque.md)):

```rut
enum JTag { Null, False, True, Num, Str, Arr, Obj }

struct Json {
    tag:  JTag;
    num: str;          // verbatim lexeme — exact round-trips, no
    str: str;          // float formatting anywhere in this file
    arr:  Vec<opaque>;
    keys: Vec<str>;
    vals: Vec<opaque>;
}
```

Storing numbers as their **verbatim lexemes** is what makes
round-trips exact: `-3e2` decodes and re-encodes as `-3e2`, with no
float formatting anywhere in the file.

### The encode half rides the std `json` package

The encode side is not private code: it is an `impl` of the std
`json` package's serialization trait, driving the package's writer.
This is the orphan rule's type-local case — json owns the trait, this
file owns `Json`, so the pair is legal exactly here
([Traits and dispatch](../core-concepts/traits-and-dispatch.md)):

```rut
impl JsonSerialize for Json {
    fn encode(self, mut w: JsonWriter) -> ?EncodeJsonError {
        when (self.tag) {
            JTag.Null -> { w.write_raw("null"); },
            JTag.False -> { w.write_raw("false"); },
            JTag.True -> { w.write_raw("true"); },
            JTag.Num -> { w.write_raw(self.num); },
            JTag.Str -> { w.write_str(self.str); },
            JTag.Arr -> {
                w.begin_array();
                for (let i = 0; i < self.arr.len(); i += 1) {
                    let ae: Json = opaque.downcast<Json>(self.arr[i]);
                    let e = ae.encode(w);
                    if (e != nil) { return e; }
                }
                w.end_array();
            },
            JTag.Obj -> {
                w.begin_object();
                for (let i = 0; i < self.keys.len(); i += 1) {
                    w.key(self.keys[i]);
                    let ve: Json = opaque.downcast<Json>(self.vals[i]);
                    let e = ve.encode(w);
                    if (e != nil) { return e; }
                }
                w.end_object();
            },
        }
        return nil;
    }
}
```

The entry surface then hands the tree to `encodeJson` and flattens
the `(str, err)` pair it answers. The decode half stays a private
cursor parser (strings iterate as codepoints, so the source is split
once into single-char strings and walked by index) — and every decode
error is an honest string value, never a trap:

```text
hex_dec("zz")   = "hex: invalid character at index 0"
b64_dec("!*")   = "base64: invalid character `!`"
json_dec("{,}") = "json: expected a key string at index 1"
```

### The host oracle, in miniature

The host keeps three-line reference implementations for the hash keys
with no canonical crate, and crates for everything else — then
compares:

```rust
// the hash-key trio with no canonical crate: three-line references
fn fnv1a32(b: &[u8]) -> u32 {
    let mut h = 0x811c9dc5u32;
    for &x in b {
        h ^= x as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}
```

The same session runs under 50 M fuel for the demos and raises to
500 M for the 64 KiB stress blob — budgets are the host's call, per
call.

## Takeaways

- The integer surface (hex literals, `u32`/`u64`, wrapping ops,
  logical shifts) is enough for MD5-through-SHA-512 — verified, not
  asserted, against independent Rust.
- `bytes` crosses the boundary directly; `opaque` is for the one
  place recursion needs breaking.
- The std `json` package is usable from day one: implement its
  serialize trait for your own type and its writer does the byte
  work ([core and the swappable packages](../reference/stdlib.md)).
- Errors are values, and the host-as-oracle pattern is the strongest
  test shape in this chapter.

The oracle pattern returns in [06 — GitHub viewer CLI](06-github-viewer-cli.md),
where the fixture is a recorded HTTP lane instead of hash crates.
