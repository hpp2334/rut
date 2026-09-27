# 06-github-viewer-cli — `rgh`, a GitHub viewer whose brain is rut

`rgh` lists and downloads files from public GitHub repositories **over
the jsDelivr CDN** — no GitHub API, no tokens, no raw.githubusercontent:

```
cargo run -p rgh -- --repo=jquery/jquery --ref=3.7.1 list
cargo run -p rgh -- --repo=jquery/jquery --ref=3.7.1 download test/data/1x1.jpg
cargo run -p rgh -- --repo=jquery/jquery --ref=3.7.1 download test/data/text.txt ./keep.txt
```

`rgh.rut` is the program: argv carving, the URL building, the tree
JSON decode, the human-size formatting, every error message and exit
code. The Rust half (`src/main.rs`) is an embedder that mounts
packages, binds host bodies, crosses the ONE argv string in, and exits
with the brain's i32 — `0` ok, `1` runtime error, `2` usage.

## The two endpoints

| data | URL |
|---|---|
| the tree (JSON, nested `files`) | `https://data.jsdelivr.com/v1/packages/gh/{owner}/{repo}@{ref}` |
| one file (bytes, verbatim) | `https://cdn.jsdelivr.net/gh/{owner}/{repo}@{ref}/{path}` |

`--ref` is REQUIRED always — a tag, branch, or sha; there is no
versions-endpoint resolution hop. Status mapping (both subcommands):
transport failure (the std lane's reserved status 0) →
`rgh: network error: …`; 404 → no such repo or ref (`list`) / no such
file (`download`), naming the host that was checked; 403 → the CDN's
rate-limit note; any other non-2xx → `rgh: CDN <n>`.

## Division of labor

| piece | where | does |
|---|---|---|
| the brain | `rgh.rut` | argv carve (hand-rolled — there is no `str.split`; the `scan`/`slice` tokenizer primitives), flag/subcommand parse, URL building, `decodeJsonBytes<Root>` over the tree JSON (`impl JsonDeserialize for Entry` + `Root` over `rut/json`'s reader), depth-first flatten, `human_size`, every message and exit code |
| the std HTTP lane | `rut/http_host` + `rut/http` (tree pkgs), bodies in `rut-std` behind its default-off `http` feature | `http::get(url) -> Response` — `status()` (0 = transport), `ok()`, `transport_error()`, `body()`, `text()` |
| the example's host rows | `rgh_host/` + `src/main.rs` | CLI I/O only: `out` (stdout line), `eprint` (stderr line), `write_file` (`std::fs::write`, io error text as the `?str` answer) |
| the embedder | `src/main.rs` | mount std + `pouch`/`nmapset`/`json` + the http pair + `rgh_host`, `assemble_peers`, compile `rgh.rut` (Impl mode), verify, bind `install_std_http` (reqwest) + the `rgh_host` rows, `verify_against`, `vm.call("rgh_main", (args,))`, exit with the i32 |

Two disclosures, both load-bearing:

- **The stdout row is `out`, not `print`.** `print` is a REMOVED core
  name whose use sites diagnose by symbol — a bare `print(..)` call
  can never compile. Same crossing, honest name.
- **`write_file`'s answer spells `any` at the decl, read as `?str` at
  the call.** The crossing set (RFC 0023 §1) refuses `?T` at a decl;
  the `http_err` precedent applies — nil = written, the text = the io
  error.

And one engine-side gap this example documented rather than dodged:
the peer-gated `impl JsonSerialize for Vec<T>` monomorphizes at every
`Vec<..>` shape in the unit — including `Vec<Entry>`, riding the
struct field — and its element `x.encode(w)` fails load-time verify
unless Entry's own `JsonSerialize` impl is compiled. `rgh.rut` carries
that impl (type-local, orphan-legal), which makes the peer gate
genuinely load-bearing: the twin dispatches the spliced `Vec<Entry>`
row itself. 02-digest hit the same gap at `T = opaque`, where the
orphan rule forbids the fix and the light mount is the only answer.

## The offline gate — the fixture lane

`cargo test -p rgh` runs the whole suite with ZERO network. The HTTP
bodies bind through `install_std_http_with(hosts, f)` where `f` maps
an exact URL to a recorded payload — the fixture map's keys ARE the
URL assertions (a wrong URL from the brain is a loud transport-error
mismatch, never a pass-through). `tests/fixtures/`:

- `tree.json` — the real `data.jsdelivr.com` tree for
  jquery/jquery@3.7.1, verbatim (351 entries, 4 depths, base64
  integrity hashes, directories without size/hash, a real 0-byte
  file);
- `tree-boundaries.json` — same API shape, hand-built to seat the
  human-size boundaries (`0 B`, `1023 B`, `1 KiB`, `1.5 KiB`,
  `1 MiB`, `1.0 MiB` — the 1048577 edge, a nonzero remainder whose
  tenth digit floors to 0 — `1 GiB`, `1.5 GiB`) and a hash-less file;
- `text.txt`, `binary.jpg` — real `cdn.jsdelivr.net` bodies; the jpg
  must land byte-verbatim;
- `notfound.json` — a real 404 body.

Coverage: every usage error (exit 2 + the stderr text), the boundary
tree's EXACT output, the real tree against an independent Rust
derivation of the format (the 02-digest oracle pattern), binary and
text downloads (default dest = the basename, explicit dest, bytes
verbatim), the write-failure path (message + exit 1, no confirmation
line), 404 / transport / 403 / 500 mappings, exit codes throughout,
and the fuel budget (the real tree lists in ~1.9M fuel against the
embedder's 250M).

The live smoke is opt-in and hits the real CDN through the embedder's
reqwest lane:

```
RGH_LIVE=1 cargo test -p rgh live_smoke
```

## The list line format

One file per line, depth-first in the API's own order (directories are
walked, never printed):

```
<repo path>\t<human size>\t<hash8>
```

- `human size` — `N B` under 1024, then KiB/MiB/GiB with one decimal
  only when the remainder is nonzero. The f-string hole is a bare
  expression (RFC 0030 §1.1) — no format specs — so the tenth is
  integer math (`rem * 10 / unit`, floor), disclosed here by the plan:
  `1536` → `1.5 KiB`, `1048577` → `1.0 MiB`.
- `hash8` — the first 8 characters of the entry's integrity hash. The
  CDN serves base64 (SRI-style, 44 chars), not hex, so these are the
  first 8 base64 chars, verbatim; a missing hash prints `-`.

## Non-goals (recorded, refused with a usage error or a message)

- no auth / private repos — data.jsdelivr.com is public-only;
- no directory downloads — a `download` path with a trailing `/` is a
  usage error (rgh moves one file at a time; a directory target that
  `fs::write` cannot open answers the io error + exit 1);
- no pagination or limits — the tree endpoint returns the whole tree;
- no async — a sequential CLI (RFC 0019's lane is separate);
- **`rut run` cannot host rgh itself** — its `rgh_host` rows are
  example-local, so only this embedder (or your own, binding the same
  three rows) can run the brain. Ordinary HTTP programs do run under
  `rut run` (rut-cli mounts the std http pair by presence).
