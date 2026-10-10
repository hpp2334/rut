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

import sieveSrc from "./sieve/mod.rut";
import quicksortSrc from "./quicksort/mod.rut";
import matrixMulSrc from "./matrix-mul/mod.rut";
import classesSrc from "./classes/mod.rut";
import closuresGenericsSrc from "./closures-generics/mod.rut";
import structsSrc from "./structs/mod.rut";
import literalsSrc from "./literals/mod.rut";
import checkedArithSrc from "./checked-arith/mod.rut";
import strViewsSrc from "./str-views/mod.rut";
import bytesSrc from "./bytes/mod.rut";
import opaqueSrc from "./opaque/mod.rut";
import whenSrc from "./when/mod.rut";
import mapsSrc from "./maps/mod.rut";
import nodeCycleSrc from "./node-cycle/mod.rut";
import treeSrc from "./tree/mod.rut";
import weakCacheSrc from "./weak-cache/mod.rut";
import typeAliasesSrc from "./type-aliases/mod.rut";

export const EXAMPLES: RutCase[] = [
  {
    id: "ex-sieve",
    name: "sieve",
    blurb: "Sieve of Eratosthenes — flat Vec<u8>/Vec<i32> primitive buffers",
    source: sieveSrc,
  },
  {
    id: "ex-quicksort",
    name: "quicksort",
    blurb: "in-place Vec<i32> mutation (handles, shared with the caller), recursion",
    source: quicksortSrc,
  },
  {
    id: "ex-matrix-mul",
    name: "matrix multiply",
    blurb: "flat Vec<f32> hot loops — unboxed buffers, no per-element refcounts",
    source: matrixMulSrc,
  },
  {
    id: "ex-classes",
    name: "classes",
    blurb: "class-method construction (new/from), Self {}, member pub + sealing",
    source: classesSrc,
  },
  {
    id: "ex-closures-generics",
    name: "closures & generics",
    blurb: "anonymous fns (block bodies — no arrow form), monomorphized generics, fn types, capture",
    source: closuresGenericsSrc,
  },
  {
    id: "ex-structs",
    name: "structs",
    blurb: "reference semantics (sharing by default), identity `==`",
    source: structsSrc,
  },
  {
    id: "ex-literals",
    name: "literals",
    blurb: "numeric suffixes, plain/raw/format strings, fixed [T], bytes buffers",
    source: literalsSrc,
  },
  {
    id: "ex-checked-arith",
    name: "checked arithmetic",
    blurb: "wrapping_* wraps two's-complement, checked_* answers the (T, bool) tuple",
    source: checkedArithSrc,
  },
  {
    id: "ex-str-views",
    name: "str views",
    blurb: "O(1) slice views (a slice IS a str), codepoints — s.code / str.from_code",
    source: strViewsSrc,
  },
  {
    id: "ex-bytes",
    name: "bytes",
    blurb: "the binary primitive — encode/decode, clone as the ONE copy",
    source: bytesSrc,
  },
  {
    id: "ex-opaque",
    name: "opaque",
    blurb: "opaque / opaque.downcast<T> -> ?T / is — erasure and checked recovery",
    source: opaqueSrc,
  },
  {
    id: "ex-when",
    name: "when",
    blurb: "when pattern expressions over enums, exhaustiveness",
    source: whenSrc,
  },
  {
    id: "ex-maps",
    name: "maps & sets",
    blurb: "the keyed-collection lane — HashMap/HashSet, keys admitted by the compile-time union bound",
    source: mapsSrc,
  },
  {
    id: "ex-node-cycle",
    name: "node cycle",
    blurb: "strong cycles keep cells alive — the program's responsibility; no collector, no weak refs yet",
    source: nodeCycleSrc,
  },
  {
    id: "ex-tree",
    name: "tree",
    blurb: "recursive structs (?Node nullable fields), composite fields as handle slots",
    source: treeSrc,
  },
  {
    id: "ex-weak-cache",
    name: "weak cache",
    blurb: "the cache/observer shape, shown with today's strong refs",
    source: weakCacheSrc,
  },
  {
    id: "ex-type-aliases",
    name: "type aliases",
    blurb: "transparent aliases, bound-only unions, inline `requires` at the call site",
    source: typeAliasesSrc,
  },
];
