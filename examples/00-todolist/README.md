# 00-todolist — a Rust app that embeds rut

The first **runnable** host example: everything else under `examples/`
is the parse-only design corpus, waiting on M2+ features. This one runs
today, on the M1 slice.

The split:

| file | role |
|---|---|
| `todolist.rut` | **the rut part** — `TodoList` class with full CRUD (`add` / `len` / `title_of` / `render` / `set_done` / `remove`) + the `entry fn` surface the host calls. No `main` — it's a library |
| `src/main.rs` | **the Rust part** — the embedder: compile → verify → drive the session through `vm.call` |
| `tests/session.rs` | asserts the whole CRUD session; runs under `cargo test --workspace` |

## Run it

```sh
cargo run -p todolist            # from the repo root
```

Output (abridged):

```
added: #1, #2, #3
set_done(#2, true)
title_of(#2) = Opt(Some(Str("implement the VM")))
title_of(42) = Opt(None)
remove(#1) = Res(Ok(Bool(true)))
remove(#1) again = Res(Err(Str("no todo #1")))
TodoList[2]
  #2 [x] implement the VM
  #3 [ ] ship the demo
second list #1: 0 todos
fuel used: 680 of Some(1000000)
```

## The design

**The host owns the session; rut owns the data.**

1. `createContainer()` → `Opaque` — rut boxes a fresh container and the
   host holds the handle (`Value::Opaque`). Opaque is the one cell an
   embedder may keep (RFC 0014).
2. `create(container)` → `u32` — a new list inside the container; the
   host keeps the handle and passes it (plus the container) to every
   later call.
3. CRUD calls cross with **plain values only**: `str`/`i32`/`bool` in,
   `i32`/`bool`/`str`/`Option`/`Result` out (RFC 0023 §2). `TodoList`
   instances never leave the VM.

## What it demonstrates

- **`entry fn`** — the host-callable surface (RFC 0035 §3), distinct from
  `pub` (import visibility for rut modules, RFC 0003 §2 — no type
  limits there). An entry's signature is checked against the crossing
  rule **at compile time**: a `TodoList` parameter on an entry is a
  source diagnostic, never a call-time failure.
- **module shape** — entries are compilation roots, so a library module
  with no `main` still emits every entry (`pub fn main` stays the
  conventional entry for scripts)
- **budgets** (RFC 0040) — the session runs under fuel + heap limits;
  embedder mistakes (wrong value shape) come back as named traps, never
  silent zeros
- the executable M1 surface — classes (`Self {}` construction,
  zero-param `new`, `-> Self`), dataclasses with field initializers,
  `Vec<T>`/`Option<T>`/`Result<T,E>`, `if/else`, `&&`/`||`, `when` with
  block arms, f-strings, `Opaque.new`/`downcast`
