# 02-digest — the byte-level one

A Rust app embedding rut, in the shape of [00-todolist](../00-todolist) and
[01-sort](../01-sort): `digest.rut` is the application, `src/main.rs` is the
embedder — and this time the embedder is also the **oracle**.

The rut side is a byte-level library, everything flowing over `bytes`
(the immutable binary primitive, RFC 0004), `str`, and `Opaque` —
the shapes RFC 0023 §2 lets cross the host boundary:

- **encodings** — hex (encode/decode, case-insensitive) and base64
  (standard + URL-safe alphabets, padding, invalid-input rejection)
- **crypto digests** — MD5, SHA-1, SHA-256, SHA-512 behind one
  `when`-on-string dispatcher (`digest("sha256", data)`)
- **hashmap hash keys** — CRC-32 (reflected, bitwise, table-free),
  FNV-1a 32/64, djb2, sdbm
- **JSON** — decode to an `Opaque` tree, encode back, round-trip;
  numbers are stored as verbatim lexemes so round-trips are exact

The host verifies everything two independent ways:

1. **canonical vectors** — RFC 1321's MD5 suite, FIPS 180-4's SHA
   examples, RFC 4648's base64 table, the CRC/FNV catalogues
2. **the crates** — `md-5`/`sha1`/`sha2`, `base64`, `crc32fast`, and
   `serde_json` cross-check every algorithm on deterministic
   pseudo-random inputs across every padding-edge length (54-57, 63-65,
   111-113, 119-121, 127-129 …), plus a 64 KiB stress blob

`digest.rut` knows nothing about the crates; only the host compares.

## Run

```
$ cargo run -q -p digests
hex([114, 117, 116, 33]) = 72757421
b64([102, 111, 111, 98, 97, 114], url=false) = Zm9vYmFy  OK
b64([102, 111, 111, 98, 97, 114], url=true) = Zm9vYmFy  OK
b64([114, 117], url=false) = cnU=  OK
digests, rut vs crates:
  md5     empty         fuel    7432  d41d8cd98f00b204e9800998ecf8427e  OK
  sha1    empty         fuel   11541  da39a3ee5e6b4b0d3255bfef95601890afd80709  OK
  sha256  empty         fuel   18495  e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  OK
  sha512  empty         fuel   26460  cf83e135...7af927da3e  OK
  md5     "abc"         fuel    7432  900150983cd24fb0d6963f7d28e17f72  OK
  sha1    "abc"         fuel   11541  a9993e364706816aba3e25717850c26c9cd0d89d  OK
  sha256  "abc"         fuel   18495  ba7816bf...f20015ad  OK
  sha512  "abc"         fuel   26460  ddaf35a1...4ca49f  OK
  md5     1 KiB lcg(42) fuel   109256  d6c1961991b0106647e36ca4ba12d345  OK
  ...
sample_doc -> {"name":"rut","version":0.2,"tags":["tiny","fast","verified"],"meta":{"ok":true,"lines":607}}
serde_json parses it: OK
hash keys over that JSON text:
  crc32   eccc5960  OK
  fnv1a32 77f26123  OK
  fnv1a64 8ba697141da02e83  OK
  djb2    9a7c023bbbc1aa33  OK
  sdbm    0b6bf47d919e2bb0  OK
json_roundtrip(tricky) = {"a":[1,2.5,-3e2],...}
  semantically equal to serde_json: OK
hex_dec("zz")   = "hex: invalid character at index 0"
b64_dec("!*")   = "base64: invalid character `!`"
json_dec("{,}") = "json: expected a key string at index 1"
digest("md4")   = "unknown algorithm: md4"
every row agrees: OK
fuel used: 1007145 of Some(50000000)
```

## What it demonstrates

- **the integer surface is enough for real algorithms** — hex literals,
  `u32`/`u64`, the wrapping family `&+ &* &<<` (RFC 0004 §3), and
  signedness-correct `>>`. SHA-512 is the showcase: its 64-bit
  rotations need `>>` to be a *logical* shift on `u64` (a bug fixed in
  this repo right before this example) and `&<<` to truncate to the
  operand width
- **`bytes` crosses directly** (RFC 0023 §2, RFC 0004) — no opaque
  wrapper needed for byte payloads; `Opaque` appears exactly once,
  boxing the recursive JSON tree
- **payloadless enums + dataclasses build a tagged union** (RFC 0006):
  `JTag` + `Json` with children as `Vec<Opaque>` — recursion through
  RFC 0014's escape hatch
- **errors are values** — malformed hex/base64/JSON and unknown
  algorithm names come back as `Result.err` strings, not traps
- **budgets are the host's call** (RFC 0040) — the demo runs under
  50 M fuel; the 64 KiB stress test raises the same session to 500 M
- **the FNV/djb2/sdbm trio never shifts 64-bit words** — a deliberate
  demonstration that `&*`/`&+`/`^` alone carry the classic hash keys

## Limitations (honest ones)

- JSON `\uXXXX` escapes are rejected with a clear error (rut strings
  iterate `char`s and there is no codepoint→`char` constructor yet);
  UTF-8 content itself round-trips fine
- JSON strings do not validate raw control characters < 0x20, and the
  encoder escapes only `" \ \n \r \t`
- base64 decode is permissive about padding position beyond "no data
  after `=`" and "length % 4 != 1"
