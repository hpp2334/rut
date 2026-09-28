/**
 * The classics: real, runnable rut programs — the exact files the
 * repo's native gate compiles and runs (`crates/rut-cli/tests/
 * playground.rs`), read raw so the playground edits live sources,
 * not string copies.
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

export const EXAMPLES: RutCase[] = [
  {
    id: "ex-sieve",
    name: "sieve",
    blurb: "Sieve of Eratosthenes — flat Vec<u8>/Vec<i32> primitive buffers",
    rfcs: "0005",
    source: sieveSrc,
  },
  {
    id: "ex-quicksort",
    name: "quicksort",
    blurb: "in-place Vec<i32> mutation (handles, shared with the caller), recursion",
    rfcs: "0005",
    source: quicksortSrc,
  },
  {
    id: "ex-matrix-mul",
    name: "matrix multiply",
    blurb: "flat Vec<f32> hot loops — unboxed buffers, no per-element refcounts",
    rfcs: "0005",
    source: matrixMulSrc,
  },
  {
    id: "ex-classes",
    name: "classes",
    blurb: "class-method construction (new/from), Self {}, member pub + sealing",
    rfcs: "0010",
    source: classesSrc,
  },
  {
    id: "ex-closures-generics",
    name: "closures & generics",
    blurb: "anonymous fns (block bodies — RFC 0013 has no arrow form), monomorphized generics, fn types, capture",
    rfcs: "0013",
    source: closuresGenericsSrc,
  },
  {
    id: "ex-structs",
    name: "structs",
    blurb: "reference semantics (sharing by default), identity `==`",
    rfcs: "0009, 0044",
    source: structsSrc,
  },
  {
    id: "ex-literals",
    name: "literals",
    blurb: "numeric suffixes, plain/raw/format strings, fixed [T], bytes buffers",
    rfcs: "0005, 0007",
    source: literalsSrc,
  },
  {
    id: "ex-checked-arith",
    name: "checked arithmetic",
    blurb: "wrapping_* wraps two's-complement, checked_* answers the (T, bool) tuple",
    rfcs: "0032 §1.1, 0004 §3",
    source: checkedArithSrc,
  },
  {
    id: "ex-str-views",
    name: "str views",
    blurb: "O(1) slice views (a slice IS a str), codepoints — s.code / str.from_code",
    rfcs: "0042, 0004",
    source: strViewsSrc,
  },
  {
    id: "ex-bytes",
    name: "bytes",
    blurb: "the binary primitive — encode/decode, clone as the ONE copy (RFC 0044)",
    rfcs: "0044 §3 §4, 0004",
    source: bytesSrc,
  },
  {
    id: "ex-opaque",
    name: "opaque",
    blurb: "opaque / opaque.downcast<T> -> ?T / is — erasure and checked recovery",
    rfcs: "0014",
    source: opaqueSrc,
  },
  {
    id: "ex-when",
    name: "when",
    blurb: "when pattern expressions over enums, exhaustiveness",
    rfcs: "0008",
    source: whenSrc,
  },
  {
    id: "ex-maps",
    name: "maps & sets",
    blurb: "the keyed-collection lane — HashMap/HashSet, keys admitted by the compile-time union bound",
    rfcs: "0043 §A5, 0023 §2",
    source: mapsSrc,
  },
  {
    id: "ex-node-cycle",
    name: "node cycle",
    blurb: "strong cycles keep cells alive — the program's responsibility; no collector, no weak refs yet",
    rfcs: "0017 §2",
    source: nodeCycleSrc,
  },
  {
    id: "ex-tree",
    name: "tree",
    blurb: "recursive structs (?Node nullable fields), composite fields as handle slots",
    rfcs: "0009",
    source: treeSrc,
  },
  {
    id: "ex-weak-cache",
    name: "weak cache",
    blurb: "the cache/observer shape, shown with today's strong refs",
    rfcs: "0017 §1",
    source: weakCacheSrc,
  },
  {
    id: "ex-type-aliases",
    name: "type aliases",
    blurb: "transparent aliases, bound-only unions, inline `requires` at the call site",
    rfcs: "0043",
    source: typeAliasesSrc,
  },
];
