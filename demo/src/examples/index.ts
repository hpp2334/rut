/**
 * The classics (RFC 0041 §3): real, runnable rut programs — the exact
 * files the repo's gate test compiles and runs (`crates/rut-cli/tests/
 * playground.rs`), read raw so the playground edits live sources,
 * not string copies. Each case's `expected` lines are INLINE data: the
 * retired `*.expected` sidecars' bytes carried VERBATIM into the block
 * below (md5 receipts in docs/demo-no-sidecars-survey.md §1.1), and
 * enforced by both gates — the smoke through wasm, playground.rs
 * natively — so neither copy can silently rot.
 *
 * Order here is the playground order: algorithms first (the classics),
 * then the language-surface tours, then the memory shapes.
 */

import type { RutCase } from "../cases";

import sieveSrc from "./sieve.rut";
import quicksortSrc from "./quicksort.rut";
import matrixMulSrc from "./matrix-mul.rut";
import classesSrc from "./classes.rut";
import closuresGenericsSrc from "./closures-generics.rut";
import structsSrc from "./structs.rut";
import literalsSrc from "./literals.rut";
import checkedArithSrc from "./checked-arith.rut";
import strViewsSrc from "./str-views.rut";
import bytesSrc from "./bytes.rut";
import opaqueSrc from "./opaque.rut";
import whenSrc from "./when.rut";
import mapsSrc from "./maps.rut";
import nodeCycleSrc from "./node-cycle.rut";
import treeSrc from "./tree.rut";
import weakCacheSrc from "./weak-cache.rut";
import typeAliasesSrc from "./type-aliases.rut";

/** the sidecars' exact shape: one leading newline (after the backtick)
 *  and trailing newlines are the template's delimiters, then line-split */
function lines(block: string): string[] {
  return block.replace(/^\n/, "").replace(/\n+$/, "").split("\n");
}

export const EXAMPLES: RutCase[] = [
  {
    id: "ex-sieve",
    name: "sieve",
    blurb: "Sieve of Eratosthenes — flat Vec<u8>/Vec<i32> primitive buffers",
    rfcs: "0005",
    source: sieveSrc,
    // every block below is the retired sidecar's bytes pasted VERBATIM,
    // content at COLUMN 0 on purpose: indenting the template would add
    // bytes to the DATA (drift by typography). No block needs an escape
    // (zero backticks/`${`/backslashes in the corpus — survey §1.1);
    // quicksort's one line ends with a LOAD-BEARING trailing space (the
    // engine emits it; both gates diff it) — do not strip it.
    expected: lines(`
25 primes up to 100, last=97
`),
  },
  {
    id: "ex-quicksort",
    name: "quicksort",
    blurb: "in-place Vec<i32> mutation (handles, shared with the caller), recursion",
    rfcs: "0005",
    source: quicksortSrc,
    expected: lines(`
sorted: 1 2 2 3 5 7 8 9 
`),
  },
  {
    id: "ex-matrix-mul",
    name: "matrix multiply",
    blurb: "flat Vec<f32> hot loops — unboxed buffers, no per-element refcounts",
    rfcs: "0005",
    source: matrixMulSrc,
    expected: lines(`
out[0]=21 out[last]=107
`),
  },
  {
    id: "ex-classes",
    name: "classes",
    blurb: "class-method construction (new/from), Self {}, member pub + sealing",
    rfcs: "0010",
    source: classesSrc,
    expected: lines(`
count=2 area=12
`),
  },
  {
    id: "ex-closures-generics",
    name: "closures & generics",
    blurb: "anonymous fns (block bodies — RFC 0013 has no arrow form), monomorphized generics, fn types, capture",
    rfcs: "0013",
    source: closuresGenericsSrc,
    expected: lines(`
add=3 area=3.1415927 sum=6
head=10 name=a
`),
  },
  {
    id: "ex-structs",
    name: "structs",
    blurb: "reference semantics (sharing by default), identity `==`",
    rfcs: "0009, 0044",
    source: structsSrc,
    expected: lines(`
len=6.324555320336759 color=16711935 area=6
same=true distinct=false fresh.x=7
`),
  },
  {
    id: "ex-literals",
    name: "literals",
    blurb: "numeric suffixes, plain/raw/format strings, fixed [T], bytes buffers",
    rfcs: "0005, 0007",
    source: literalsSrc,
    expected: lines(`
a=10 e=1.5 d64=1.5 ch=h p.x=1 zero[0]=9 len=3 bin=64
`),
  },
  {
    id: "ex-checked-arith",
    name: "checked arithmetic",
    blurb: "wrapping_* wraps two's-complement, checked_* answers the (T, bool) tuple",
    rfcs: "0032 §1.1, 0004 §3",
    source: checkedArithSrc,
    expected: lines(`
wrap=4 under=255
over=(44, false)
ok=(255, true)
under=(255, false)
mul=(44, false)
i32 top=(-2147483648, false) wrap=-2147483648
roundtrip=0
`),
  },
  {
    id: "ex-str-views",
    name: "str views",
    blurb: "O(1) slice views (a slice IS a str), codepoints — s.code / str.from_code",
    rfcs: "0042, 0004",
    source: strViewsSrc,
    expected: lines(`
word=world len=5 eq=true
iterated=5 reslice=or
chars=6 octets=7
accent=é
code=104 back=h round=true
`),
  },
  {
    id: "ex-bytes",
    name: "bytes",
    blurb: "the binary primitive — encode/decode, clone as the ONE copy (RFC 0044)",
    rfcs: "0044 §3 §4, 0004",
    source: bytesSrc,
    expected: lines(`
round=true octets=8 chars=8
header=RUT scratch.len=16
alias same content: true
clone same content: true
octets=6 chars=5
`),
  },
  {
    id: "ex-opaque",
    name: "opaque",
    blurb: "opaque / opaque.downcast<T> -> ?T / is — erasure and checked recovery",
    rfcs: "0014",
    source: opaqueSrc,
    expected: lines(`
point 1 2
sour? true wrong? true
is str: false
one cell: 9 5 5 9
same session: true
distinct boxes: false
value 5
str box misses i32: true
3 boxes; first is Point: false
vec 1
`),
  },
  {
    id: "ex-when",
    name: "when",
    blurb: "when pattern expressions over enums, exhaustiveness",
    rfcs: "0008",
    source: whenSrc,
    expected: lines(`
small
`),
  },
  {
    id: "ex-maps",
    name: "maps & sets",
    blurb: "the keyed-collection lane — HashMap/HashSet, keys admitted by the compile-time union bound",
    rfcs: "0043 §A5, 0023 §2",
    source: mapsSrc,
    expected: lines(`
rut=3 runs=1
replace=false rut=9
removed=true len=2 has=false
miss is nil: true
a=10 b=20
miss is nil: true
a=11 len=2
first=true again=false len=1
has x=true has z=false
`),
  },
  {
    id: "ex-node-cycle",
    name: "node cycle",
    blurb: "strong cycles keep cells alive — the program's responsibility; no collector, no weak refs yet",
    rfcs: "0017 §2",
    source: nodeCycleSrc,
    expected: lines(`
head.next alive: true
`),
  },
  {
    id: "ex-tree",
    name: "tree",
    blurb: "recursive structs (?Node nullable fields), composite fields as handle slots",
    rfcs: "0009",
    source: treeSrc,
    expected: lines(`
nodes=15
`),
  },
  {
    id: "ex-weak-cache",
    name: "weak cache",
    blurb: "the cache/observer shape, shown with today's strong refs",
    rfcs: "0017 §1",
    source: weakCacheSrc,
    expected: lines(`
held: true id=1
miss: false
`),
  },
  {
    id: "ex-type-aliases",
    name: "type aliases",
    blurb: "transparent aliases, bound-only unions, inline `requires` at the call site",
    rfcs: "0043",
    source: typeAliasesSrc,
    expected: lines(`
trip=1500 plain=1500 ridge/trench kind=trench
`),
  },
];
