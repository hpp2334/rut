# Errors and optionality

rut has no exceptions you catch, no `Result` enum, and no `null`. Two
mechanisms cover everything:

- **Absence** is `nil` on a nullable `?T`.
- **Failure** is a value in the second slot of a pair — the `(T, err)`
  answer channel.
- **Bugs** — broken contracts — trap loudly and immediately
  (`panic`, `assert`, a nil deref, an out-of-bounds index).

The design background lives in
[everything is a value](../core-concepts/everything-is-a-value.md).

## `?T` and `nil`: absence

`?T` is a nil-able cell. A plain `T` always holds a value; `?T` is that
value or `nil`:

```rut
struct Node {
    value: i32;
    left: ?Node;      // one word — recursive shapes are legal
    right: ?Node;
}
```

Lookups answer `?T`: a hit is the value, a miss is `nil`. Deref is
automatic at every use — `p.x`, `p[i]`, `for (let x of p)`, arithmetic
— and dereferencing a `nil` **traps** `NilDeref`, so guard first:

```rut
let hit = scores.get("rut");      // ?i32
if (hit != nil) {
    n = hit + 1;                  // a hit auto-unwraps for `+`
}
```

`p == nil` compares against the null. There is no flow-typing: after
the guard, hand the value to a typed binding when you want a plain `T`:

```rut
let err = map_response(resp, "no such repo or ref", "data.jsdelivr.com");
if (err != nil) {
    let why: str = err;           // the plain str behind the nullable
    eprint(why);
    return 1;
}
```

`nil` in an expected-`?T` position just works: `let p: ?Node = nil;`,
or a `left: nil` field in a literal.

## The `(T, err)` answer channel

Functions that can fail return a pair. The convention is uniform:

- **empty err + a value** — success;
- **empty err + `nil`** — a legitimate "not found";
- **non-empty err** — failure.

The simplest err is a `str` message (empty string means success):

```rut
fn hex_dec(s: str) -> (bytes, str) {
    if (s.len() % 2 != 0) {
        return (bytes.zeroed(0), "hex: odd-length input");
    }
    // ... decode ...
    return (out.freeze(), "");
}
```

Nullable halves make both slots explicit — this is the standard
library's own shape, `(?T, ?E)`:

```rut
// json's entry points
fn decodeJson<T requires JsonDeserialize>(s: str) -> (?T, ?DecodeJsonError);
fn encodeJson<T requires JsonSerialize>(v: T) -> (?str, ?EncodeJsonError);
```

The caller destructures and checks the err half first:

```rut
let (n, e) = decodeJson<i64>("42");
if (e == nil) {
    log.info(f"n={n}");
} else {
    let why = e;
    log.info(f"decode failed at {why.at}");
}
```

Integer arithmetic's checked ladder uses the same idea in miniature:
`x.checked_add(y)` answers `(T, bool)` where `.1` is `false` exactly
when the result escaped the width — see [values and
variables](values-and-variables.md).

## Building an error type

For errors a caller might want to *branch on*, pair a payloadless enum
(the kind) with a fixed-field struct (the details). That is the shape
`json` uses:

```rut
pub enum DecodeErrorKind { Unexpected, Truncated, InvalidUtf8, WrongType, Depth, Trailing }

pub struct DecodeJsonError {
    kind: DecodeErrorKind;
    at: i64;        // codepoint offset in the input
    got: str;       // the offending text
    expected: str;  // what the schema asked for here
}
```

The caller picks: branch on `kind`, or just render the fields:

```rut
let (bad, be) = decodeJson<i64>("[1,2,3]");
if (be != nil) {
    let why = be;
    // rejected at 0: got '[', wanted an i64
    log.info(f"rejected at {why.at}: got '{why.got}', wanted {why.expected}");
}
```

Enums have no payloads — the struct carries the data. This split keeps
every error a plain value: copy it, store it in a log, put a location
next to it, test it.

## `panic` and `assert`: for bugs, not for flow

```rut
panic("Rect: negative extents");        // abort with a message
assert(total == expected, "checksum");  // abort when the condition is false
```

Use these when continuing would be a lie: a broken invariant, an
impossible state, a caller error that is a programming mistake. Data
problems — a bad byte in a file, a missing key, a network error — are
*values* and belong in the answer channel, so callers can recover.

A trap's message and a stack trace (opt-in via `capture_stacktrace()`)
land in the host's diagnostics — see
[diagnostics, traces, and symbolication](../reference/diagnostics.md).

## Erasure when you truly need it

When a value must cross a boundary that cannot name its type — host
handles, heterogeneous boxes — `opaque(v)` seals it and
`opaque.downcast<T>(o) -> ?T` recovers it (`nil` on a wrong type,
never a trap):

```rut
let box1 = opaque(Point { x: 1, y: 2 });
let p = opaque.downcast<Point>(box1);   // ?Point
when (p != nil) {
    true -> { log.info(f"point {p.x} {p.y}"); },
    else -> { log.info("point: nil"); },
}
```

`x is T` probes the box without recovering it. This is rut's only
`any`-shaped value, and it can do nothing until recovered — see
[the reference on opaque](../reference/opaque.md).

## Put it together

```rut
use json::{ decodeJson, encodeJson };
use ink::{ Logger };

pub fn main() {
    let log = Logger.new("json");

    // a successful decode: (value, nil)
    let (n, e) = decodeJson<i64>("42");
    if (e == nil) {
        log.info(f"n={n}");
    } else {
        let why = e;
        log.info(f"decode failed at {why.at}: {why.expected}");
    }

    // encode answers the same shape: (?str, ?EncodeJsonError)
    let (s, ee) = encodeJson<[i64]>([1, 2, 3]);
    if (ee == nil) {
        let text = s;
        log.info(f"s={text}");
    } else {
        log.info("encode failed");
    }

    // a bad decode: the (nil, err) half answers the failure
    let (bad, be) = decodeJson<i64>("[1,2,3]");
    if (be == nil) {
        log.info("unexpected success");
    } else {
        let why = be;
        log.info(f"rejected at {why.at}: got '{why.got}', wanted {why.expected}");
    }
}
```

```text
n=42
s=[1,2,3]
rejected at 0: got '[', wanted an i64
```

Next: [modules and packages](modules.md).
