# 00 — Todolist

The first runnable host example, and the template for the rest of the
chapter: a Rust program embeds rut, compiles a rut **library** (there
is no `main` in it), and drives the library through its `entry fn`
surface. The split is two files — `todolist.rut` owns the data and the
logic as a `TodoList` class with full CRUD; `src/main.rs` owns the
session: compile, verify, then a scripted conversation of `vm.call`s.
The one-sentence design rule: **the host owns the session, rut owns
the data.**

## Run it

```sh
cargo run -p todolist        # from the repo root
cargo test -p todolist       # the same session asserted as a test
```

The run prints an abridged transcript like this:

```text
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

## Code tour

### The data: a class with a fields-only body

`examples/00-todolist/todolist.rut` — the type body is fields only;
methods live in an inherent `impl` block
([Structs](../reference/structs.md),
[Classes and constructors](../reference/classes.md)). Unannotated
members are module-private. The block below is the class verbatim —
`new`, `add`, and `remove` — closed with a `main` so it runs on its
own; the example file itself has no `main` — it is a library the host
drives through its `entry fn` surface:

```rut
use ink::{ Logger };
use pouch::{ Vec };

struct Todo {
    id: i32;
    title: str;
    done: bool = false;      // field initializer — `done` may be omitted
}

pub class TodoList {
    items: Vec<Todo>;         // unannotated members = module-private
    next_id: i32;
}

impl TodoList {
    pub fn new() -> Self {
        return Self { items: Vec.new(), next_id: 1 };
    }

    // ---- CREATE ----

    pub fn add(mut self, title: str) -> i32 {
        let id = self.next_id;
        self.items.push(Todo { id: id, title: title });
        self.next_id += 1;
        return id;
    }

    // ---- DELETE ----

    pub fn remove(mut self, id: i32) -> bool {
        let kept: Vec<Todo> = Vec.new();
        let mut removed = false;
        for (let t of self.items) {
            when (t.id == id) {
                true -> { removed = true; },   // drop the row
                else -> { kept.push(t); },     // keep everything else
            }
        }
        if (removed) {
            self.items = kept;
        }
        return removed;
    }
}

entry fn main() {
    let log = Logger("todos");
    let mut list = TodoList.new();
    list.add("implement the VM");
    list.add("ship the demo");
    let first = list.remove(1);
    let second = list.remove(1);
    log.info(f"remove(#1) = {first}, again = {second}, left: {list.items.len()}");
}
```

```text
remove(#1) = true, again = false, left: 1
```

Note `mut self`: writing through a class handle requires a `mut`
binding head, the same law [01 — Sort](01-sort.md) exercises on plain
vecs — `add` and `remove` both declare it. `remove` filters through a
`when`: the matching row is dropped, everything else is kept, and the
list is rewritten only if something was actually removed. The read
side (not shown) is ordinary code — `len`, `title_of` (which returns
`""` for a miss; the example pairs it with `has_title` because there
is no `?.` sugar), and a `render` built from f-strings.

### The handle: one opaque box around the world

Instances of `TodoList` can never cross into Rust — the crossing set
admits primitives, `str`, `bytes`, and `opaque` only, and `entry fn`
signatures are checked against that rule **at compile time** (see
[the host boundary](../core-concepts/host-boundary.md)). So the
library boxes its whole container once and the host holds the handle:

```rut
struct Lists {
    lists: Vec<TodoList>;    // handle = index; handles stay stable while held
}

fn at(c: opaque, h: u32) -> TodoList {
    let lists = opaque.downcast<?Lists>(c);
    return lists.lists[h as i32];
}

entry fn createContainer() -> opaque {
    let ls: ?Lists = Lists { lists: Vec.new() };
    return opaque(ls);
}

entry fn create(c: opaque) -> u32 {
    let k = opaque.downcast<?Lists>(c);
    k.lists.push(TodoList.new());
    return (k.lists.len() - 1) as u32;
}
```

Two things to take away. First, `entry fn` marks the host-callable
surface — distinct from `pub`, which is use-visibility for other rut
modules and carries no crossing limits
([Modules and visibility](../reference/modules-and-visibility.md)).
Second, erasure is the `opaque(v)` type-call and recovery is
`opaque.downcast<T>(o) -> ?T` — a wrong box answers `nil`, never a
trap ([opaque — erasure and downcast](../reference/opaque.md)). The
internal helper `at` is an ordinary `fn` and may speak in full types.

### The embedder: typed calls, budgets, and honest errors

`examples/00-todolist/src/main.rs` mounts core and the `pouch`
package (for `Vec`), compiles in one call, verifies the binary, and
then talks to the module through typed closures:

```rust
let c: rut_vm::OpaqueRef = vm.call("createContainer", ()).unwrap();
let list: u32 = vm.call("create", (c.clone(),)).unwrap();

let mut add = |title: &str| -> i32 {
    vm.call::<_, i32>("add", (c.clone(), list, title)).unwrap()
};
let vmf = add("implement the VM");
let demo = add("ship the demo");
```

The session runs under explicit budgets — fuel and a heap limit — so
a runaway module fails loudly instead of hanging the host
([Resource limits and fuel](../reference/resource-limits.md)):

```rust
let limits = rut_vm::interp::Limits {
    fuel: Some(1_000_000),
    heap_limit_bytes: Some(4 * 1024 * 1024),
    interrupt_every: 1024,
};
```

And the transcript's `remove(#1) again = Res(Err(...))` line is the
VM's typed view of a `bool` answer — the second remove returns
`false`, and the debug print shows the value shape the crossing
produces ([Value boundary and borrows](../reference/value-boundary.md)).

## Takeaways

- `entry fn` is the host-callable surface; its signatures are checked
  against the crossing rule at compile time, so a bad plan is a
  diagnostic, never a call-time surprise.
- State lives rut-side in one `opaque` container the host holds and
  re-passes; plain values cross, instances never do.
- Erasure is `opaque(v)`; recovery is `opaque.downcast<T> -> ?T`.
- Budgets (fuel + heap) are the embedder's call, and every embedder
  mistake comes back as a named trap.
- The same file works as a library: entries are compilation roots, so
  a module with no `main` still emits every entry.

Next: [01 — Sort](01-sort.md) scales the pattern to five algorithms
and shows what recursion costs in fuel. The crossing rules behind this
page are in [the host boundary](../core-concepts/host-boundary.md).
