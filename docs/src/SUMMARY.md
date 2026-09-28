# Summary

- [Introduction](README.md)

---

# Quick Start

- [Installation](quick-start/installation.md)
- [Your first rut program](quick-start/first-program.md)
- [The playground](quick-start/playground.md)

# Tutorial

- [Values and variables](tutorial/values-and-variables.md)
- [Control flow and when](tutorial/control-flow.md)
- [Functions, closures, and generics](tutorial/functions.md)
- [Structs, enums, and classes](tutorial/types.md)
- [Traits and impl blocks](tutorial/traits.md)
- [Errors and optionality](tutorial/errors.md)
- [Modules and packages](tutorial/modules.md)
- [Async: tasks, workers, and channels](tutorial/async.md)
- [The standard library](tutorial/stdlib.md)

# Core Concepts

- [Design goals](core-concepts/design-goals.md)
- [Everything is a value](core-concepts/everything-is-a-value.md)
- [Reified types and layouts](core-concepts/reified-types.md)
- [Traits and dispatch](core-concepts/traits-and-dispatch.md)
- [The async model](core-concepts/async-model.md)
- [Memory: the Rc heap, weak refs, and cycles](core-concepts/memory.md)
- [The host boundary](core-concepts/host-boundary.md)
- [The bytecode VM](core-concepts/the-vm.md)

# Examples

- [Examples index](examples/index.md)
- [00 — Todolist](examples/00-todolist.md)
- [01 — Sort](examples/01-sort.md)
- [02 — Digest](examples/02-digest.md)
- [03 — Plugin](examples/03-plugin.md)
- [04 — Custom async](examples/04-custom-async.md)
- [05 — Todolist web](examples/05-todolist-web.md)
- [06 — GitHub viewer CLI](examples/06-github-viewer-cli.md)
- [The playground corpus](examples/playground-corpus.md)

# Reference

---

## Language

- [Lexical structure](reference/lexical-structure.md)
- [Modules and visibility](reference/modules-and-visibility.md)
- [Primitive types](reference/primitive-types.md)
- [Builtin generic types](reference/builtin-generic-types.md)
- [Enums](reference/enums.md)
- [Literals and inference](reference/literals-and-inference.md)
- [Control flow and when](reference/control-flow.md)
- [Structs](reference/structs.md)
- [Classes and constructors](reference/classes.md)
- [Rc, dispose, and identity](reference/rc-dispose-identity.md)
- [Traits and dispatch](reference/traits.md)
- [Functions, closures, and generics](reference/functions-closures-generics.md)
- [opaque — erasure and downcast](reference/opaque.md)
- [Reified types and layout](reference/reified-types.md)
- [String slicing and views](reference/string-views.md)
- [Type aliases and union bounds](reference/type-aliases.md)
- [By-reference and nullable](reference/by-reference-and-nullable.md)

## Runtime and memory

- [The Rc heap and destructors](reference/rc-heap.md)
- [Weak references and the cycle collector](reference/weak-refs.md)
- [The VM heap](reference/vm-heap.md)
- [Resource limits and fuel](reference/resource-limits.md)

## Async and concurrency

- [Async and await](reference/async.md)
- [Tasks](reference/tasks.md)
- [The host futures bridge](reference/host-futures.md)
- [Workers and channels](reference/workers-and-channels.md)

## Embedding and interop

- [Embedding and native modules](reference/embedding.md)
- [Value boundary and borrows](reference/value-boundary.md)
- [repr(C) struct interop](reference/repr-c.md)
- [Host fns and declaration files](reference/host-fns.md)
- [Native containers API surface](reference/native-containers.md)
- [Templates — f"..." across the boundary](reference/templates.md)
- [Reflection](reference/reflection.md)

## Standard library

- [core and the swappable packages](reference/stdlib.md)

## Toolchain

- [The frontend](reference/frontend.md)
- [The compiler pipeline](reference/compiler.md)
- [Typed bytecode](reference/typed-bytecode.md)
- [Module binary and verification](reference/module-binary.md)
- [VM core](reference/vm-core.md)
- [Loading and the embed loop](reference/loading.md)
- [Diagnostics, traces, and symbolication](reference/diagnostics.md)
- [Module bundles](reference/bundles.md)

## Packages and tooling

- [Project structure and rut.toml](reference/project-structure.md)
- [Dependency kinds](reference/dependency-kinds.md)
- [The rut CLI](reference/cli.md)
