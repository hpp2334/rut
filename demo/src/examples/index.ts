/**
 * The classics (RFC 0041 §3): real, runnable rut programs — the exact
 * files the repo's gate test compiles and runs (`crates/rut-cli/tests/
 * playground.rs`), raw-imported so the playground edits live sources,
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
import dataclassesSrc from "./dataclasses.rut";
import dataclassesExpected from "./dataclasses.expected";
import literalsSrc from "./literals.rut";
import literalsExpected from "./literals.expected";
import opaqueSrc from "./opaque.rut";
import opaqueExpected from "./opaque.expected";
import whenSrc from "./when.rut";
import whenExpected from "./when.expected";
import nodeCycleSrc from "./node-cycle.rut";
import nodeCycleExpected from "./node-cycle.expected";
import treeSrc from "./tree.rut";
import treeExpected from "./tree.expected";
import weakCacheSrc from "./weak-cache.rut";
import weakCacheExpected from "./weak-cache.expected";

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
    blurb: "arrows, monomorphized generics, fn types, capture by value",
    rfcs: "0013",
    source: closuresGenericsSrc,
    expected: lines(closuresGenericsExpected),
  },
  {
    id: "ex-dataclasses",
    name: "dataclasses",
    blurb: "reference semantics (aliasing by default), own divergence, identity ==",
    rfcs: "0009, 0011, 0016",
    source: dataclassesSrc,
    expected: lines(dataclassesExpected),
  },
  {
    id: "ex-literals",
    name: "literals",
    blurb: "numeric suffixes, plain/raw/format strings, fixed Array<T, N>",
    rfcs: "0005, 0007",
    source: literalsSrc,
    expected: lines(literalsExpected),
  },
  {
    id: "ex-opaque",
    name: "Opaque",
    blurb: "Opaque.new / downcast<T> / is — erasure and checked recovery",
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
    id: "ex-node-cycle",
    name: "node cycle",
    blurb: "reference cycles and the collector — the shape Weak<T> exists for",
    rfcs: "0017 §2",
    source: nodeCycleSrc,
    expected: lines(nodeCycleExpected),
  },
  {
    id: "ex-tree",
    name: "tree",
    blurb: "recursive dataclasses (Option<Node>), composite fields as handle slots",
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
];
