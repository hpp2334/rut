# 06 — GitHub viewer CLI

`rgh` lists and downloads files from public GitHub repositories over
the jsDelivr CDN — no GitHub API, no tokens. Its **brain is a rut
async free fn**: argv carving, the URL building, the tree JSON
decode, the human-size formatter, every message and exit code run in
the VM. The Rust half is a pure embedder: mount packages, bind host
bodies, launch the brain, pump the driving loop to idle, exit with
the brain's i32 (`0` ok, `1` runtime error, `2` usage). This is the
worked example for the std `http` lane's redesigned face —
`HttpClient`, builder-style construction, and async-only send with
true streaming.

## Run it

```sh
cargo run -p rgh -- --repo=jquery/jquery --ref=3.7.1 list
cargo run -p rgh -- --repo=jquery/jquery --ref=3.7.1 download test/data/1x1.jpg
cargo test -p rgh                  # the offline suite — zero network
RGH_LIVE=1 cargo test -p rgh live_smoke   # opt-in live CDN check
```

`list` prints one line per file, depth-first in the API's own order
(directories are walked, never printed):

```text
<repo path>\t<human size>\t<hash8>
```

The size ladder is `N B` under 1024, then KiB/MiB/GiB with one
decimal only when the remainder is nonzero (`1536` → `1.5 KiB`,
`1048577` → `1.0 MiB`); the last column is the first 8 characters of
the entry's base64 integrity hash, or `-` when absent. The offline
suite rides a **fixture lane** keyed on method + URL with a virtual
clock — the fixture map's keys ARE the assertions — and covers every
usage error, the boundary tree's exact output, streamed binary
downloads verified byte-verbatim, wire deaths, and status mappings.

## Code tour

### The brain is async; the host launches and pumps

`rgh.rut` — the entry point is one `boot` fn: the host crosses argv
in as a single `\n`-joined string (no arg lists in the crossing set),
and `boot` launches the brain with the standard launcher
([launched futures](../reference/launched-futures.md)):

```rut
entry fn boot(args: str) -> nil {
    launch_future(rgh_main(args));
}
```

The embedder then pumps the driving loop to idle — real reqwest
workers settle the completers from their threads, and the process's
one-way `exit` row fires mid-pump:

```rust
loop {
    match vm.run_ready() {
        Ok(_) => {}
        Err(t) => {
            eprintln!("rgh: trap: {} — {}", t.name(), t.msg);
            std::process::exit(1);
        }
    }
    if vm.pending_tasks() == 0 {
        // the brain retired WITHOUT exiting — a brain bug: loud
        eprintln!("rgh: the brain retired without exit (a brain bug)");
        std::process::exit(1);
    }
    std::thread::sleep(std::time::Duration::from_millis(2));
}
```

The awaits live at the fetch sites and only there — everything else
(argv carving with the `scan`/`slice` tokenizer primitives, flags,
JSON, formatting) is the same sync code it always was.

### The http face: build a request, await the send

The std `http` package is **async-only and unsuffixed**: only the
operations that really wait are async points, everything else is sync
construction sugar. The five verbs are build sugars (`get`/`post`/
`put`/`patch`/`del` — the DELETE verb spells `del`; the wire still
sees canonical `DELETE`), and `send()` is THE async point, resolving **at
headers** — the wire body stays unread
([the async model](../core-concepts/async-model.md)):

```rut
async fn do_list(client: HttpClient, owner: str, repo: str, rf: str) -> i32 {
    let url = f"https://data.jsdelivr.com/v1/packages/gh/{owner}/{repo}@{rf}";
    let resp = await client.get(url).build().send();
    let err = map_response(resp, "no such repo or ref", "data.jsdelivr.com");
    if (err != nil) {
        let why: str = err;
        eprint(why);
        return 1;
    }
    let (tree, e) = decodeJsonBytes<Root>(await resp.body());
```

`list` is the **drain** lane: one `body()` await pulls the whole
tree, and the JSON decode goes through the std `json` package's
reader with `impl JsonDeserialize for Entry` in this file. Status
mapping is one function: `status() == 0` is the reserved transport
verdict, 404 names the host that was checked, 403 is the rate-limit
note, anything else non-2xx is the bare status.

### The download: true streaming, chunk by chunk

`download` walks a `ByteStream` — `next()` answers one chunk per
await (nil = EOF-or-failed; the sticky `error()` names which), and
each chunk lands through the **sync** `append_file` host row. The
destination is truncated once first, so a stale file never leaks its
tail into a fresh download:

```rut
    let stream = resp.byte_stream();
    let mut total: i32 = 0;
    while (true) {
        let c = await stream.next();
        if (c == nil) {
            // EOF — or a mid-read failure? the sticky error names it
            let rerr = stream.error();
            if (rerr != nil) {
                let why: str = rerr;
                eprint(f"rgh: {dest}: {why}");
                return 1;
            }
            break;
        }
        let chunk: bytes = c;
        let werr: ?str = append_file(dest, chunk);
        if (werr != nil) {
            let why: str = werr;
            eprint(f"rgh: {dest}: {why}");
            return 1;
        }
        total += chunk.len();
    }
```

Bounded memory end to end, and every failure on the data path is a
message plus an exit code — never a panic, never a trap. The lane's
one-shot law is worth knowing: per response it is `body()` **xor**
`byte_stream()`, and a late/second taker degrades (an empty drain, a
dead reader) rather than trapping.

### The readbacks, on the response handle

The `Response` class wraps the host's opaque handle with sync
readbacks — status, the 2xx test, the transport text, and the sticky
mid-read failure ([the std
packages](../reference/stdlib.md)):

```rut
impl Response {
    /// the HTTP status word — 0 means the transport failed (0 is never
    /// a real status; any real status, 4xx/5xx included, is not one)
    pub fn status(self) -> i32 { return http_status(self.r); }

    /// the 2xx test
    pub fn ok(self) -> bool {
        let s = self.status();
        return s >= 200 && s < 300;
    }

    /// the transport-failure text — nil unless status is 0
    pub fn transport_error(self) -> ?str { return http_err(self.r); }

    /// the sticky mid-read failure — nil until one fails, then
    /// non-nil forever; meaningful after awaiting `body()`/`next()`
    pub fn read_error(self) -> ?str { return http_read_err(self.r); }
```

### The embedder's mount list

`src/main.rs` shows the whole program closure in one table — the
collections, json, and the http pair, then `assemble_peers` runs the
peer gate so json's impl-only integration groups mount because the
collections are in the closure
([Dependency kinds](../reference/dependency-kinds.md)):

```rust
const MOUNT_DIRS: &[&str] = &[
    "rut/pouch",
    "rut/nmapset",
    "rut/json",
    "rut/http_host",
    "rut/http",
];
```

The HTTP bodies install through `http::pkg()` (the reqwest lane);
the example's own rows are CLI I/O only — `out` (stdout), `eprint`,
the file pair, and `exit` — declared in
the example-local `rgh_host` decl package and verified against the
bindings at boot ([Host fns and declaration
files](../reference/host-fns.md)). One consequence: `rut run` cannot
host rgh itself — its rows are example-local — but ordinary HTTP
programs do run under `rut run`, which mounts the std http pair by
presence.

## Takeaways

- **An async rut program in production shape**: the brain awaits at
  the fetch sites only; the embedder launches, pumps to idle, and
  exits with the brain's code.
- **Headers-first send + one-shot body/stream** is the whole mental
  model of the std http lane — drain JSON in one await, stream files
  chunk by chunk.
- **Failures are values end to end**: transport errors, 404s, wire
  deaths, and io errors all become messages plus exit codes.
- **The fixture lane is the test gate**: recorded replies keyed on
  method + URL, deterministic chunks on a virtual clock, zero
  network.
- The peer gate is load-bearing here: mounting the collections is
  what makes json's `Vec<T>` serde impls compile into this unit.

The lane's declarations live in the std tree; for the runner behind
`rut run`, see [the rut CLI](../reference/cli.md).
