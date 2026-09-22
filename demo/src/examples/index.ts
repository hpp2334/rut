/**
 * The classics (RFC 0041 §3): real, runnable rut programs — the exact
 * files the repo's gate test compiles and runs (`crates/rut-cli/tests/
 * playground.rs`), read raw so the playground edits live sources,
 * not string copies. Each `NAME.expected` sidecar is the program's
 * actual output, not a hand-written promise.
 *
 * Order here is the playground order: algorithms first (the classics),
 * then the language-surface tours, then the memory shapes.
 */

import type { RutCase } from "../cases";

import sieveSrc from "./sieve.rut";
import sieveExpected from "./sieve.expected";
import quicksortSrc from "./quicksort.rut";
import quicksortExpected from "./quicksort.expected";
import matrixMulSrc from "./matrix-mul.rut";
import matrixMulExpected from "./matrix-mul.expected";
import classesSrc from "./classes.rut";
import classesExpected from "./classes.expected";
import closuresGenericsSrc from "./closures-generics.rut";
import closuresGenericsExpected from "./closures-generics.expected";
import structsSrc from "./structs.rut";
import structsExpected from "./structs.expected";
import literalsSrc from "./literals.rut";
import literalsExpected from "./literals.expected";
import checkedArithSrc from "./checked-arith.rut";
import checkedArithExpected from "./checked-arith.expected";
import strViewsSrc from "./str-views.rut";
import strViewsExpected from "./str-views.expected";
import bytesSrc from "./bytes.rut";
import bytesExpected from "./bytes.expected";
import opaqueSrc from "./opaque.rut";
import opaqueExpected from "./opaque.expected";
import whenSrc from "./when.rut";
import whenExpected from "./when.expected";
import mapsSrc from "./maps.rut";
import mapsExpected from "./maps.expected";
import nodeCycleSrc from "./node-cycle.rut";
import nodeCycleExpected from "./node-cycle.expected";
import treeSrc from "./tree.rut";
import treeExpected from "./tree.expected";
import weakCacheSrc from "./weak-cache.rut";
import weakCacheExpected from "./weak-cache.expected";
import typeAliasesSrc from "./type-aliases.rut";
import typeAliasesExpected from "./type-aliases.expected";

function lines(expected: string): string[] {
  return expected.replace(/\n+$/, "").split("\n");
}

export const EXAMPLES: RutCase[] = [
  {
    id: "ex-sieve",
    name: "sieve",
    blurb: "Sieve of Eratosthenes — flat Vec<u8>/Vec<i32> primitive buffers",
    rfcs: "0005",
    source: sieveSrc,
    expected: lines(sieveExpected),
  },
  {
    id: "ex-quicksort",
    name: "quicksort",
    blurb: "in-place Vec<i32> mutation (handles, shared with the caller), recursion",
    rfcs: "0005",
    source: quicksortSrc,
    expected: lines(quicksortExpected),
  },
  {
    id: "ex-matrix-mul",
    name: "matrix multiply",
    blurb: "flat Vec<f32> hot loops — unboxed buffers, no per-element refcounts",
    rfcs: "0005",
    source: matrixMulSrc,
    expected: lines(matrixMulExpected),
  },
  {
    id: "ex-classes",
    name: "classes",
    blurb: "class-method construction (new/from), Self {}, member pub + sealing",
    rfcs: "0010",
    source: classesSrc,
    expected: lines(classesExpected),
  },
  {
    id: "ex-closures-generics",
    name: "closures & generics",
    blurb: "anonymous fns (block bodies — RFC 0013 has no arrow form), monomorphized generics, fn types, capture",
    rfcs: "0013",
    source: closuresGenericsSrc,
    expected: lines(closuresGenericsExpected),
  },
  {
    id: "ex-structs",
    name: "structs",
    blurb: "reference semantics (sharing by default), identity `==`",
    rfcs: "0009, 0044",
    source: structsSrc,
    expected: lines(structsExpected),
  },
  {
    id: "ex-literals",
    name: "literals",
    blurb: "numeric suffixes, plain/raw/format strings, fixed [T], bytes buffers",
    rfcs: "0005, 0007",
    source: literalsSrc,
    expected: lines(literalsExpected),
  },
  {
    id: "ex-checked-arith",
    name: "checked arithmetic",
    blurb: "wrapping_* wraps two's-complement, checked_* answers the (T, bool) tuple",
    rfcs: "0032 §1.1, 0004 §3",
    source: checkedArithSrc,
    expected: lines(checkedArithExpected),
  },
  {
    id: "ex-str-views",
    name: "str views",
    blurb: "O(1) slice views (a slice IS a str), codepoints — s.code / str.from_code",
    rfcs: "0042, 0004",
    source: strViewsSrc,
    expected: lines(strViewsExpected),
  },
  {
    id: "ex-bytes",
    name: "bytes",
    blurb: "the binary primitive — encode/decode, clone as the ONE copy (RFC 0044)",
    rfcs: "0044 §3 §4, 0004",
    source: bytesSrc,
    expected: lines(bytesExpected),
  },
  {
    id: "ex-opaque",
    name: "opaque",
    blurb: "opaque / opaque.downcast<T> -> ?T / is — erasure and checked recovery",
    rfcs: "0014",
    source: opaqueSrc,
    expected: lines(opaqueExpected),
  },
  {
    id: "ex-when",
    name: "when",
    blurb: "when pattern expressions over enums, exhaustiveness",
    rfcs: "0008",
    source: whenSrc,
    expected: lines(whenExpected),
  },
  {
    id: "ex-maps",
    name: "maps & sets",
    blurb: "the keyed-collection lane — HashMap/HashSet/PrimMapI64, keys admitted by the compile-time union bound",
    rfcs: "0043 §A5, 0023 §2",
    source: mapsSrc,
    expected: lines(mapsExpected),
  },
  {
    id: "ex-node-cycle",
    name: "node cycle",
    blurb: "strong cycles keep cells alive — the program's responsibility; no collector, no weak refs yet",
    rfcs: "0017 §2",
    source: nodeCycleSrc,
    expected: lines(nodeCycleExpected),
  },
  {
    id: "ex-tree",
    name: "tree",
    blurb: "recursive structs (?Node nullable fields), composite fields as handle slots",
    rfcs: "0009",
    source: treeSrc,
    expected: lines(treeExpected),
  },
  {
    id: "ex-weak-cache",
    name: "weak cache",
    blurb: "the cache/observer shape, shown with today's strong refs",
    rfcs: "0017 §1",
    source: weakCacheSrc,
    expected: lines(weakCacheExpected),
  },
  {
    id: "ex-type-aliases",
    name: "type aliases",
    blurb: "transparent aliases, bound-only unions, inline `requires` at the call site",
    rfcs: "0043",
    source: typeAliasesSrc,
    expected: lines(typeAliasesExpected),
  },
];
