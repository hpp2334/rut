# 05 — Todolist web

One page app — a todolist with a simulated server — where **every DOM
move and every timer goes through ten declared host crossings**, and
the page's brain is a rut project of **two packages**. `ui` is the
framework: an atom store (the jotai shape, in rut), a widget type, a
keyed diff, and a component vocabulary. `biz` is the domain: the
todolist machine expressed as store mutations, plus the app shell.
The host — web-sys on wasm32 in the browser, a fake-DOM twin on
native in the tests — owns the loop; rut owns the state. Pure rut: no
event loop, no async keywords on the rut side; every asynchronous
fact enters through a door named for its event.

## Run it

```sh
cargo test -p todolist-web       # the twin + the law (95 tests), from the repo root
cd examples/05-todolist-web
node tests/e2e-browser.mjs       # the through-the-artifact gate (node tier + browser tier)
```

`cargo test` runs the whole story on the **fake-DOM twin** — a
HashMap element tree with real DOM semantics and a virtual timer
clock — over the same rut sources the browser runs. The e2e script
builds nothing silently and skips nothing silently: tier 1 (plain
node) drives the real wasm artifact through the loader's own ABI on a
fake DOM; tier 2 (when geckodriver + Firefox exist) types and clicks
the live page.

To open the page yourself:

```sh
cd examples/05-todolist-web
cargo build -p todolist-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir gen --out-name web_host \
  ../../target/wasm32-unknown-unknown/release/todolist_web.wasm
python3 -m http.server        # then open http://localhost:8000/
```

What you'll see: type a title — the status line echoes the typing and
the counts never move (the draft is not a counts dependency). Press
Add — an italic *pending* line paints immediately, the counts line
flips to `1 in flight`, and ~400 ms later the committed row replaces
it. Toggle a row — the title goes italic for ~250 ms, then the check
fills and the title strikes through. The row animates because the
keyed diff **kept the node alive** — nothing is cleared and rebuilt,
ever.

## Code tour

### The ABI is the page: `main` plus the doors

`rut/biz/app.rut` — the boot turn builds the app container and
returns it as an `opaque` (rut has no mutable module state, so the
host holds the state and re-passes it every turn — the
[00 — Todolist](00-todolist.md) pattern at page scale):

```rut
entry fn main() -> opaque {
    let boot: World = world_boot("booted — type a title, press Add");
    let root = AppRoot {
        world: boot,
        t1: t1_mount("app"),
    };
    paint(root);
    return opaque(root);
}
```

Every asynchronous fact enters through a door named for its event —
`on_click`/`on_input` for DOM events, `on_timer` for the clock. Each
door re-passes the container and answers the entry-err pair:
`(opaque(c), "")` on a clean turn, `(nil, why)` on a soft failure the
pump reports *while keeping the page alive*.

```rut
fn dom_event(mut r: AppRoot, id: str, detail: str) -> (?opaque, str) {
    let booked = t1_event(r.t1, id, detail);
    let req = opaque.downcast<Req>(booked);
    if (req != nil) {
        tim_after(req.latency, req.tag);
    }
    paint(r);
    return (opaque(r), "");
}
```

The "server" is a request table, not a scheduler: adding never
touches the list — the machine books a request and the app books one
`tim_after(latency, tag)`; the list changes only when the timer's
turn runs the answer mutation (per-kind latency, so deadline-ordered
commits are visible across kinds).

### The ten crossings

`web.d.rut` (flat at the example root, registered by hand in both
lanes) declares the whole host surface. Elements cross as `opaque`
handles; everything else is strings, ints, and bools — no app names,
no data shipping, no callbacks ([the host
boundary](../core-concepts/host-boundary.md)):

```rut
pub host fn ui_get(id: str) -> opaque;
pub host fn ui_create(tag: str) -> opaque;
pub host fn ui_set_text(el: opaque, text: str);
pub host fn ui_attr(el: opaque, name: str, value: str);
pub host fn ui_append(parent: opaque, child: opaque);
pub host fn ui_remove(parent: opaque, child: opaque) -> bool;
pub host fn ui_clear(el: opaque);
pub host fn ui_set_input_value(el: opaque, v: str);
pub host fn ui_listen(el: opaque, event: str) -> i64;
pub host fn tim_after(ms: i64, tag: str);
```

A wiring bug is loud: a missing id traps (`web::ui_get: no element
'#x'`), a kind mismatch names both sides. The host never grew an
app-specific crossing — the entire widget system fits over these ten
rows, unchanged.

### The store: jotai's shape over private traits

`rut/ui/store.rut` is the kernel. Handles are keys, not objects —
`Source<T>`, `Derived<T>`, `Mutation<A, R>` carry ids and have no
logic of their own beyond routing through their store. The machinery
traits are **module-private**, so the domain package cannot name them
even to import them — which is how "a derived cell is not writable"
is enforced at the type level
([Traits and dispatch](../core-concepts/traits-and-dispatch.md)):

```rut
trait Readable<T> {
    fn atom_id(self) -> u32;
    fn materialize(self, st: Store) -> nil; // first touch: the seed lands
}

trait Writable<A, R> {
    fn atom_id(self) -> u32;
}
```

Freshness is **pull-on-read**: a write bumps a generation, and a
`get` recomputes a stale derived at most once per write-set. There is
no flush step anywhere. Dependencies are **discovered, never
declared** — the `ctx.get` calls inside a derive closure ARE the
dependency list, re-recorded on every recompute. The domain's counts
line is the worked example (`rut/biz/world.rut`):

```rut
let counts$ = store.derive(fn (ctx) -> str {
    let items = ctx.get(items$);
    let reqs = ctx.get(reqs$);
    let mut open = 0;
    for (let t of items) {
        if (t.done == false) {
            open += 1;
        }
    }
    let done_n = items.len() - open;
    return f"{open} open | {done_n} done | {reqs.len()} in flight";
});
```

Writes go through mutation fns — `add_m`, `toggle_m`, `remove_m`,
`answer_m` — multi-step write programs that run in the one write
lane; the machine's counters are sources too, because with no domain
container there is nowhere else for state to live.

### Widgets are data; the diff does the DOM

`view(r)` is a pure function from app state to a `Widget` tree — it
runs no crossing and reads nothing (reads are reactive *props*,
resolved at the render door). `t1_render` diffs the new tree against
the previous one held in `T1Root` and fires only deltas: match
children by key, patch in place, re-append on reorder, retire gone
keys' listeners. Nothing changed = nothing fires
([HashMaps and sets](../reference/builtin-generic-types.md) carry the
path-keyed tables):

```rut
pub struct T1Root {
    parent: opaque; // the mounted root element
    prev: ?Widget = nil; // the previous tree — the diff's left side
    els: HashMap<str, opaque>; // path -> live handle (ops application)
    regs: HashMap<str, Ev>; // listener id (as crossed) -> the
    // widget's event wiring (a data-shaped binding)
    lids: HashMap<str, i64>; // path -> listener id (retirement)
}
```

Event wiring rides the widgets as **data, not closures** — a row's
toggle mutation carries its argument bound (`.on_row(w.toggle_row,
t.id)`), and the framework resolves a firing listener id to that
wiring. Styling is a token contract: the stylesheet keys only the
lowered `t1-*` tokens, and the app spells no class.

### Two packages, two manifests

A real project is not one file — but the package boundary is also the
privacy boundary, which is why there are exactly two
([Project structure and rut.json](../reference/project-structure.md)):

```json
// rut/biz/rut.json — the project root
{
  "name": "app",
  "entry": { "lib": "./biz.rut", "libs": ["./domain.rut", "./world.rut", "./app.rut"] },

  "deps": {
    "ui":      { "path": "../ui" },
    "pouch":   { "path": "../../../../rut/pouch" },
    "nmapset": { "path": "../../../../rut/nmapset" }
  }
}
```

`entry.libs` splices several files into ONE module (base first, array
order), so a name private to `store.rut` is visible to
`components.rut` and to nothing outside `ui` — single-file privacy,
kept at package scale. `ui` is marked `inline = true` (its generic
exports splice by law), and the native lane mounts the *directory*
— the manifest, not a Rust fn, is the module list. The wasm lane
mounts the same packages by hand as a mirror, and a test pins both
lanes to byte-identical binaries.

## Takeaways

- The page ABI is `main` plus doors named for their events; state
  lives in the container the host re-passes every turn.
- Ten thin crossings carried an entire reactive framework — the host
  stayed generic and never learned the word "todo".
- The store is jotai's shape in rut: discovered deps, pull-on-read
  freshness, mutations as the write lane, and type-level
  non-writability through private traits.
- Widgets are values; the keyed diff is the only thing that speaks
  DOM, and patch-in-place is what makes the UI animatable.
- Soft failures are data (`(nil, why)` and the pump keeps draining);
  a panic alone kills the page, loud.

The async shape this page *simulates* (the request table + timers) is
the real thing in [06 — GitHub viewer CLI](06-github-viewer-cli.md);
the store's building blocks are in
[the swappable packages](../reference/stdlib.md).
