# Async: tasks, workers, and channels

rut's concurrency is **pull-based**. An `async fn` compiles into a
*future* — a cold value that runs nothing until something drives it.
There are no promises that start on creation, no microtask queue, no
implicit scheduling: the host owns time, and code progresses only when
a driving loop pumps it. The model is described in
[the async model](../core-concepts/async-model.md), with the full
surface in [async and await](../reference/async.md) and
[tasks](../reference/tasks.md).

## `async fn` and `await`

An `async fn` declares its context — `cx: RunContext` — as the *first
parameter*. Calling it runs nothing: it returns a cold future. `await`
is the one in-body suspension point, and it consumes the future:

```rut
use core::{ RunContext };

async fn countdown(cx: RunContext, log: Logger, n: u32) {
    let mut i = n;
    while (i > 0) {
        log.info(f"t-{i}");
        await sleep(500);       // park here; the loop resumes the frame
        i -= 1;
    }
    log.info("lift-off");
}
```

The cx is engine-minted at call sites the way `self` is — you never
pass it when *calling* an async fn:

```rut
launch_future(countdown(log, 3));   // no cx — the engine supplies it
```

Two consume paths, and exactly one per future:

- **`await fut`** — drive it from this async fn's body. Legal only
  inside an `async fn`, and only for engine-woven futures (async-fn
  results and `sleep`).
- **`launch_future(fut)`** — hand it to the driving loop and keep a
  receipt. Returns a `LaunchedFutureHandle<T>`, which is *not* a future:
  it cannot be awaited or re-launched. Its `abort()` flags cancellation;
  the frame notices at its next checkpoint and runs its cleanup.

`sleep(ms)` is the one built-in pender — a `Future<nil>` that parks its
frame until the VM's clock reaches the deadline.

Both `launch_future` and `sleep` come from the `async_host` package:

```rut
use async_host::{ launch_future, sleep };
```

## Who drives?

You never write a driving loop — the *embedder* does. The `rut run` CLI
is an embedder: after `main` returns it pumps the loop — drain ready
frames, advance the virtual clock to the next timer deadline, repeat —
until nothing is pending. So a program launches work from `main` and
the output simply appears:

```sh
rut run countdown.rut
```

```text
t-3
t-2
t-1
lift-off
```

Calling an async function and awaiting it in the same body is the
straight-line case; launching is for concurrency — independent work
that overlaps through the loop's interleaving.

## Talking to the network: the `http` package

The standard `http` package is async-only where work actually waits.
Everything else — the client, the verbs, the builder — is sync
construction sugar:

```rut
use http::{ HttpClient };

let client = HttpClient.new();
let resp = await client.get(url).build().send(cx);
```

The shape, step by step:

- `client.get(url)` (or `request()`, `post`, `put`, `patch`, `del`)
  hands back a `RequestBuilder`; `.method(..)`, `.url(..)`,
  `.header(k, v)` (repeatable), `.body(bytes)` chain on it.
- `.build()` freezes a re-sendable `Request`.
- **`.send(cx)` is the async point.** It resolves at *headers* — the
  wire body is still unread.
- From the `Response`, take one of two readbacks:
  - `await resp.body(cx)` — drain the remaining body in one future;
  - `resp.byte_stream()` then `await stream.next(cx)` — one chunk per
    future, `nil` at end-of-stream, for bounded-memory downloads.

Errors come back as data, not traps: status `0` is reserved for
transport failure (`resp.transport_error()` carries the reason), any
real status — 4xx and 5xx included — is a normal response
(`resp.ok()` is the 2xx test), and a mid-read wire death surfaces as
`nil` from `next()` with the reason in `stream.error()`.

A complete worker from the repository's GitHub-viewer example — send,
then drain:

```rut
async fn do_list(cx: RunContext, client: HttpClient, owner: str, repo: str, rf: str) -> i32 {
    let url = f"https://data.jsdelivr.com/v1/packages/gh/{owner}/{repo}@{rf}";
    let resp = await client.get(url).build().send(cx);
    if (resp.status() == 0) {
        let why: str = resp.transport_error();
        eprint(f"rgh: network error: {why}");
        return 1;
    }
    if (!resp.ok()) {
        eprint(f"rgh: CDN {resp.status()}");
        return 1;
    }
    let (tree, e) = decodeJsonBytes<Root>(await resp.body(cx));
    if (e != nil) {
        let why: DecodeJsonError = e;
        eprint(f"rgh: bad tree JSON at {why.at}");
        return 1;
    }
    // ... print the tree ...
    return 0;
}
```

The stream lane walks chunks — send, then loop:

```rut
let resp = await client.get(url).build().send(cx);
let stream = resp.byte_stream();
while (true) {
    let c = await stream.next(cx);
    if (c == nil) {
        let rerr = stream.error();
        if (rerr != nil) { return 1; }   // failed mid-read
        break;                            // clean end of stream
    }
    let chunk: bytes = c;
    total += chunk.len();
}
```

## `await select`, join, and structured scopes

Racing futures (`await select { fut1 -> .., fut2 x -> .. }`) and
joining a launched future's value (`await handle`) are spelled in the
grammar but not in this build — the compiler gates them. Cancellation
*is* here: `handle.abort()` flags the frame, and the probe at its next
checkpoint unwinds it deterministically, running cleanup in reverse
declaration order. See [tasks](../reference/tasks.md) for the
roadmap.

## Workers and channels

For true parallelism, a host can spawn **workers** — separate VMs on
separate threads with separate heaps. There is no shared memory: all
data crosses typed channels, and what may cross is a checked rule —
primitives and `str` copy; a cell (vec, record, class instance)
transfers when exclusively held, else deep-copies; endpoints
(`Sender`/`Receiver`) transfer; **closures do not cross**. A trap in a
worker kills only that worker. The API is a host facility
(`spawn_worker`, `Channel<T>`), so its shape depends on your embedder —
see [workers and channels](../reference/workers-and-channels.md).

## Put it together

```rut
use core::{ RunContext };
use async_host::{ launch_future, sleep };
use ink::{ Logger };

async fn countdown(cx: RunContext, log: Logger, n: u32) {
    let mut i = n;
    while (i > 0) {
        log.info(f"t-{i}");
        await sleep(500);
        i -= 1;
    }
    log.info("lift-off");
}

pub fn main() {
    let log = Logger.new("countdown");
    launch_future(countdown(log, 3));
}
```

```text
t-3
t-2
t-1
lift-off
```

Next: [the standard library](stdlib.md).
