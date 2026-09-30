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
packages, binds host bodies, launches the brain, pumps the driving
loop to idle, and exits with the brain's i32 — `0` ok, `1` runtime
error, `2` usage.

## The brain is async

`rgh_main` is an ASYNC free fn (`async fn rgh_main(cx: RunContext,
args: str) -> i32` — the [async](../../docs/src/reference/async.md) shape; the cx is
engine-minted,
never passed). The embedder's one entry is `boot`:

```rut
entry fn boot(args: str) -> nil {
    launch_future(rgh_main(args));
}
```

The host crosses argv in, `boot` launches, and the embedder pumps the
driving loop (`run_ready` + a wall-clock spin until
`pending_tasks()` hits 0 — the
[loading](../../docs/src/reference/loading.md) lane; real reqwest
workers settle the completers from their threads). The code crosses
out through the sync `exit` row — the process's one-way door, fired
mid-pump; the brain keeps the `-> i32` discipline beside it.

The fetch sites await — and ONLY the fetch sites:

```rut
let resp = await client.get(url).build().send(cx);   // resolves at HEADERS
let (tree, e) = decodeJsonBytes<Root>(await resp.body(cx));  // list: one drain
let c = await stream.next(cx);                       // download: chunk per await
```

Everything else — argv carve, flags, URLs, JSON, formatting — is the
same sync code it always was.

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
| the brain | `rgh.rut` | argv carve (hand-rolled — there is no `str.split`; the `scan`/`slice` tokenizer primitives), flag/subcommand parse, URL building, `decodeJsonBytes<Root>` over the tree JSON (`impl JsonDeserialize for Entry` + `Root` over `rut/json`'s reader), depth-first flatten, `human_size`, every message and exit code; the awaits live at the fetch sites |
| the std HTTP lane | `rut/http_host` + `rut/http` (tree pkgs), bodies in `rut-std` behind its default-off `http` feature | the async-only face: `HttpClient.new()`, the verbs as build sugars (`get`/`post`/`put`/`patch`/`del`), the builder chain, `build()`, and the async points — `send(cx)` (headers), `body(cx)` (the drain), `byte_stream()` + `next(cx)` (chunk per await); `status()` (0 = transport), `ok()`, `transport_error()`, `read_error()` |
| the example's host rows | `rgh_host/` + `src/main.rs` | CLI I/O only: `out` (stdout line), `eprint` (stderr line), `write_file` (truncate-or-create), `append_file` (append-or-create — one streamed chunk per call), `exit` (the one-way door) |
| the embedder | `src/main.rs` | mount std + the async pair + `pouch`/`nmapset`/`json` + the http pair + `rgh_host`, `assemble_peers`, compile `rgh.rut` (Impl mode), verify, install `http::pkg()` (reqwest) + `async_host::pkg()` + the `rgh_host` rows, `verify_against`, `vm.call("boot", (args,))`, pump to idle, exit with the carried code |

The disk-write split is the streaming shape: `download` TRUNCATES the
dest through `write_file` once (a stale file never leaks its tail
into a fresh download), then walks `byte_stream()` — `await
next(cx)` per chunk, each landing through the sync `append_file` row.
Disk writes are sync embedder rows: blocking the loop briefly per
chunk is the CLI norm, disclosed. The chunks arrive one per await
(bounded memory end to end), and the byte count in the confirmation
line is the sum of the arrived chunks.

## The laws this example rides (the std lane's contract)

- **send resolves at HEADERS** — the wire body stays unread; the
  status and the mapping run before any body byte is pulled.
- **body xor stream (the one-shot law)** — `list` drains
  (`resp.body(cx)`, one future), `download` streams
  (`resp.byte_stream()`, chunk per await). A response never gives
  both; a late/second taker DEGRADES (an empty drain / a dead reader
  whose next is an immediate EOF) — never a trap.
- **mid-read failures are data** — a wire death mid-download answers
  nil from `next`, the sticky `stream.error()` names it, rgh prints
  `rgh: <dest>: <why>` and exits 1 with the arrived prefix already
  on disk. Never a panic, never a trap.
- **transport failure is status 0** — the reserved verdict; its text
  always rides `transport_error()`.
- **cancellation is best-effort** — rgh never aborts a fetch; the
  general law (a worker thread finishes its blocking read, the late
  result is discarded) is disclosed at the lane.

Two naming disclosures, both load-bearing:

- **The stdout row is `out`, not `print`.** `print` is a REMOVED core
  name whose use sites diagnose by symbol — a bare `print(..)` call
  can never compile. Same crossing, honest name.
- **The DELETE verb is `del`.** `delete` is a REMOVED word in rut's
  grammar (the dynamic-property statement) — the fifth verb spells
  `del`; the wire still sees the canonical `DELETE`.

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
bodies bind through `http::pkg_with(f)` where `f` maps
(method, url, headers, body) to a recorded `FixtureReply` — status
plus the CHUNK PLAN — or `Err(message)` for a transport failure (the
status-0 lane). The fixture map's keys ARE the assertions (a wrong
URL or a wrong method from the brain is a loud transport-error
mismatch, never a pass-through), and the chunks arrive on the
VIRTUAL CLOCK: the send at tick 0, chunk i at tick i+1, the terminal
(EOF or the mid-read failure) one tick later — the deterministic
small chunks the streaming cases assert on. The returned `HttpFixture`
handle is the test loop's clock side. `tests/fixtures/`:

- `tree.json` — the real `data.jsdelivr.com` tree for
  jquery/jquery@3.7.1, verbatim (351 entries, 4 depths, base64
  integrity hashes, directories without size/hash, a real 0-byte
  file);
- `tree-boundaries.json` — same API shape, hand-built to seat the
  human-size boundaries (`0 B`, `1023 B`, `1 KiB`, `1.5 KiB`,
  `1 MiB`, `1.0 MiB` — the 1048577 edge, a nonzero remainder whose
  tenth digit floors to 0 — `1 GiB`, `1.5 GiB`) and a hash-less file;
- `text.txt`, `binary.jpg` — real `cdn.jsdelivr.net` bodies; the jpg
  must land byte-verbatim through the streamed lane;
- `notfound.json` — a real 404 body.

Coverage: every usage error (exit 2 + the stderr text), the boundary
tree's EXACT output, the real tree against an independent Rust
derivation of the format (the 02-digest oracle pattern), streamed
downloads (binary verbatim through truncate+appends in deterministic
chunks, text to an explicit dest, one append row per chunk), the
write-failure paths (truncate and mid-stream append: message + exit
1, no confirmation line), the mid-read wire death (exit 1, the sticky
text, the arrived prefix on disk), 404 / transport / 403 / 500
mappings, the ONE-SHOT LAW (both taker orders through a dedicated
probe riding the same mounts — drain-then-mint degrades to a dead
reader, stream-then-drain degrades to empty), exit codes throughout,
and the fuel budget (the real tree lists in ~1.9M fuel against the
embedder's 250M).

The live smoke is opt-in and hits the real CDN through the embedder's
reqwest lane (the recording `exit`, the wall-clock pump):

```
RGH_LIVE=1 cargo test -p rgh live_smoke
```

It uses `--repo=jquery/jquery --ref=3.7.1` — that pair RESOLVES on
the CDN; `jqlang` does NOT (the batch's recorded lesson).

## The list line format

One file per line, depth-first in the API's own order (directories are
walked, never printed):

```
<repo path>\t<human size>\t<hash8>
```

- `human size` — `N B` under 1024, then KiB/MiB/GiB with one decimal
  only when the remainder is nonzero. The f-string hole is a bare
  expression ([the frontend](../../docs/src/reference/frontend.md)) —
  no format specs — so the tenth is
  integer math (`rem * 10 / unit`, floor), disclosed here by the plan:
  `1536` → `1.5 KiB`, `1048577` → `1.0 MiB`.
- `hash8` — the first 8 characters of the entry's integrity hash. The
  CDN serves base64 (SRI-style, 44 chars), not hex, so these are the
  first 8 base64 chars, verbatim; a missing hash prints `-`.

## Non-goals (recorded, refused with a usage error or a message)

- no auth / private repos — data.jsdelivr.com is public-only;
- no directory downloads — a `download` path with a trailing `/` is a
  usage error (rgh moves one file at a time; a write that cannot open
  answers the io error + exit 1);
- no pagination or limits — the tree endpoint returns the whole tree;
- no concurrency — the brain awaits its fetches sequentially
  (join/select is the [tasks](../../docs/src/reference/tasks.md) lane);
- **`rut run` cannot host rgh itself** — its `rgh_host` rows are
  example-local, so only this embedder (or your own, binding the same
  rows) can run the brain. Ordinary HTTP programs do run under
  `rut run` (the manifest's `[deps]` http row mounts the std pair, and
  the CLI drives the async loop).
