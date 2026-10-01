#!/usr/bin/env python3
"""The po fold — merge the fresh rut.pot into docs/po/zh_CN.po.

Merge law (the GLOSSARY contract, after the 432ace9 lesson):
- entries in POT order; the header rides from the existing zh_CN.po;
- an UNCHANGED msgid keeps its msgstr byte-for-byte;
- a CHANGED msgid whose diff is exactly the mechanical cutover
  (rut.json → rut.jsonc, wire v7→v9 / v8→v10) carries the old msgstr
  through the SAME mechanical substitution — the zh text moves with
  the file name and the wire numbers, nothing else;
- everything else new/changed: FRESH below (written against
  docs/po/GLOSSARY.md) or "" — the English fallback BY DESIGN, never
  a stale zh sentence. Code blocks fold to "" (the untranslated set
  is code-only — 596f61a's doctrine).

Run:  python3 scripts/fold-zh-po.py            (writes docs/po/zh_CN.po)
Then: python3 scripts/po_assert.py             (THE ASSERTION)
"""
import sys

sys.path.insert(0, 'scripts')
from fold_po_lib import parse, quote, mech, esc

POT = 'docs/po/rut.pot'
PO = 'docs/po/zh_CN.po'

# Fresh translations for the strings this sweep changed non-mechanically
# (terminology per docs/po/GLOSSARY.md; code spans byte-identical;
# markdown structure load-bearing). The vocabulary purge's zh shape
# rides MECH (数据类 → 结构体 alongside dataclass → struct); everything
# below is new or non-mechanically changed prose. The three new ```rut
# blocks fold to "" — the untranslated set is code-only, by doctrine.
FRESH = {
 "An enum is a distinct named type over integer constants. No payloads and no computed members — where another language would use a union of literal strings, rut uses an enum:":
  "枚举是建立在整型常量之上的一个独立命名类型。没有载荷，也没有计算型成员——换作别的语言会在字面量字符串的并集上做文章的地方，rut 用枚举：",

 "`when` over an enum must be exhaustive — every member, or an `else` arm (see [control flow and when](control-flow.md)). Enums render as their member name in format strings. Enums take `impl` blocks — non-self methods on the name, `self` methods on a value, and trait impls (`impl Iterator<E> for Light` makes `for (let v of l)` walk) — see [enums](../reference/enums.md).":
  "对枚举的 `when` 必须穷尽——覆盖每个成员，或带一个 `else` 分支（参见[控制流与 when](control-flow.md)）。枚举在格式化字符串里渲染为成员名。枚举可以带 `impl` 块——非 self 方法挂在枚举名上调用，`self` 方法挂在值上调用，还有 trait impl（`impl Iterator<E> for Light` 让 `for (let v of l)` 走起来）——参见[枚举](../reference/enums.md)。",

 "Declare with `struct`, construct with a literal — anywhere in a function body, nested inside other literals. Construction is the literal; there is no constructor gate:":
  "用 `struct` 声明，用字面量构造——函数体内的任何位置，或嵌在其他字面量里。构造就是字面量；没有构造器门禁：",

 "All fields are public, always — member visibility in a struct is a compile error. Privacy is what classes are for. Methods live in `impl` blocks with real visibility — `pub fn` exports cross-module, plain `fn` stays module-private (see [structs](../reference/structs.md)).":
  "所有字段永远公开——给结构体成员标注可见性是编译错误。私有正是类存在的意义。方法住在 `impl` 块里，可见性是真实的——`pub fn` 跨模块导出，不带 `pub` 的 `fn` 只在模块内可见（参见[结构体](../reference/structs.md)）。",

 "**Struct fields are always public** — the field law has no dial (see [Structs](structs.md)). **Impl-block methods take the `pub` dial** — `pub fn` exports cross-module, plain `fn` is module-private — the same law a class's methods follow, enums included (see [Structs](structs.md) and [Enums](enums.md)). Trait method signatures and impl methods are as visible as their trait (see [Traits and dispatch](traits.md)).":
  "**结构体的字段永远公开**——字段法则没有调节项（参见[结构体](structs.md)）。**impl 块的方法走 `pub` 调节项**——`pub fn` 跨模块导出，不带 `pub` 的 `fn` 只在模块内可见——与类的方法同一法则，枚举亦然（参见[结构体](structs.md)与[枚举](enums.md)）。trait 方法签名与 impl 方法的可见性与其 trait 一致（参见[trait 与分派](traits.md)）。",

 "No data payloads and no computed members — ever. Methods are an `impl` block away (see [Impl blocks](#impl-blocks)).":
  "没有数据载荷，也没有计算型成员——永远如此。方法就在一个 `impl` 块之外（参见[impl 块](#impl-blocks)）。",

 "Enums take `impl` blocks — the same three forms a struct or class takes: non-self methods (called on the enum's name), `self` methods (called on a value), and trait impls.":
  "枚举可以带 `impl` 块——与结构体和类相同的三种形式：非 self 方法（通过枚举名调用）、`self` 方法（通过值调用），以及 trait impl。",

 "A non-self method is a namespaced function: `Dial.default()`. `Self` spells the enum, so `-> Self` is the return type and `Dial.Low` mints the member.":
  "非 self 方法是一个带命名空间前缀的函数：`Dial.default()`。`Self` 拼写的就是这个枚举，所以 `-> Self` 是返回类型，`Dial.Low` 铸造出成员。",

 "A `self` method dispatches on the value: `d.flipped()` — the receiver crosses as the singleton cell it already is.":
  "`self` 方法在值上分派：`d.flipped()`——接收者本来就是单例单元，按原样跨越。",

 "Method visibility is real: `pub fn` exports cross-module, plain `fn` is module-private (see [Modules and visibility](modules-and-visibility.md)).":
  "方法可见性是真实的：`pub fn` 跨模块导出，不带 `pub` 的 `fn` 只在模块内可见（参见[模块与可见性](modules-and-visibility.md)）。",

 "Trait impls make enum values iterable — `for (let v of c)` rides the same desugar as a class's (see [The iteration protocol](traits.md)):":
  "trait impl 让枚举值可迭代——`for (let v of c)` 走与类相同的脱糖（参见[迭代协议](traits.md)）：",

 "One limit stays: `Disposal` is for structs and classes only — the engine disposes a record's cell, and an enum member is immortal.":
  "只有一条限制仍在：`Disposal` 只属于结构体和类——引擎处置的是记录的单元，而枚举成员是不朽的。",

 "Iterating form — vecs, fixed arrays, slices, strings (one-codepoint `str`s per step), `bytes` (`u8` per step), and any type with a registered `Iterator` impl — an enum value included (see [Traits and dispatch](traits.md) and [Enums](enums.md)):":
  "迭代形式 —— vec、定长数组、切片、字符串（每步一个单码点 `str`）、`bytes`（每步一个 `u8`），以及任何注册了 `Iterator` impl 的类型——枚举值也在内（参见[trait 与分派](traits.md)与[枚举](enums.md)）：",

 "Methods carry real visibility — the class rule: `pub fn` exports cross-module, plain `fn` is module-private. `Self` spells the struct, in signatures (`-> Self`) and the literal (`Self { x: 1, y: 2 }`) alike:":
  "方法带有真实可见性——类的法则：`pub fn` 跨模块导出，不带 `pub` 的 `fn` 只在模块内可见。`Self` 拼写的就是这个结构体，签名（`-> Self`）和字面量（`Self { x: 1, y: 2 }`）里都一样：",

 "**fields are always public** — no field-visibility dial (privacy needs construction control, which is the class's job — see [Classes and constructors](classes.md));":
  "**字段永远公开**——字段可见性没有调节项（私有需要构造控制，那是类的职责——参见[类与构造器](classes.md)）；",

 "**construction is the literal** — `Point { x: 1, y: 2 }` everywhere (open literal vs class-method-gated _is_ the struct/class distinction).":
  "**构造就是字面量**——`Point { x: 1, y: 2 }` 到处可用（开放字面量 vs 类方法门禁_正是_结构体/类的分界）。",

 "Everything else class-shaped is allowed — `impl` blocks, `Self`, `Disposal`. And the old \"no destructor\" limit is gone: a shared value dies exactly when its cell's refcount reaches zero, so cleanup is one `impl` away — implement `Disposal` for the type and the engine calls `dispose` at that moment (see [Rc, dispose, and identity](rc-dispose-identity.md)).":
  "其余类形态的东西都允许——`impl` 块、`Self`、`Disposal`。而旧的“没有析构器”限制已经消失：共享值恰在其单元引用计数归零时死亡，所以清理只是一个 `impl` 之遥——为该类型实现 `Disposal`，引擎便会在那一刻调用 `dispose`（参见[Rc、dispose 与同一性](rc-dispose-identity.md)）。",

 "An enum value iterates the same way: `impl Iterator<E> for Color` makes `for (let v of c)` walk whatever the impl's `iterate` emits — the desugar is the trait, the target's kind is irrelevant (see [Enums](enums.md)).":
  "枚举值以同样的方式迭代：`impl Iterator<E> for Color` 让 `for (let v of c)` 走遍 impl 的 `iterate` 所发射的一切——脱糖的是 trait，目标的种类无关紧要（参见[枚举](enums.md)）。",

 "A user record is an **array of 8-byte slots**:":
  "用户记录是一个 **8 字节槽位的数组**：",

 "user record (struct)":
  "用户记录（struct）",

 "One flat `Tok` enum for literals, punctuation, and operators. Keywords are ordinary `Ident`s; the parser matches them by interner name. **Reserved words are rejected by the lexer** with a message naming the rut replacement (`interface`, `private`, `void`, `in`); `?.` and `??` are punctuation-level rejections, a separate mechanism.":
  "一个扁平的 `Tok` 枚举覆盖字面量、标点和运算符。关键字就是普通的 `Ident`；语法分析器按驻留名匹配它们。**保留字由词法分析器拒绝**，消息会指明 rut 的替代写法（`interface`、`private`、`void`、`in`）；`?.` 与 `??` 是标点层面的拒绝，属于另一套机制。",

 # ---- the capture law (closure captures follow the ref law) ----

 "Closures capture the enclosing bindings — a closure body sees and can use the locals around it, including after the enclosing function would have returned, because captured cells stay alive as long as the closure does. The capture law has two halves: primitives (and `nil` and `fn` values) copy — the closure gets its own slot, and a reassign inside the closure never moves the original — while everything ref-headed (a `Vec`, a struct, `str`, `?T`, …) crosses as a handle to the same cell. That second half includes whole-value reassignment: a captured binding that is reassigned — either frame — shares its slot with the closure, so both sides always see the current value:":
  "闭包捕获外围绑定——闭包体看得见、用得上它周围的局部量，即使外围函数早已返回也没有关系，因为被捕获的单元只要闭包还活着就不会消失。捕获法则有两半：标量（以及 `nil` 和 `fn` 值）按复制走——闭包拿到自己的槽位，在闭包内重新赋值绝不会移动原绑定——而一切以引用为头的值（`Vec`、结构体、`str`、`?T`……）都以句柄跨越：穿过被捕获句柄的写入抵达同一个单元，整体重新赋值也是如此——一个既被捕获又被重新赋值的绑定会晋升为隐藏的共享槽位单元，两个帧在整个绑定作用域内保持联动（先暂存再重赋值、闭包内重赋值、`for..of` 循环变量，都遵循这同一条法则）。",

 "`for (let v of it)` desugars to `it.iterate(emit)` with a synthetic closure, and the capture law covers it like every closure: scalars copy (a rebind inside the loop stays local), ref-headed bindings share their slot — reassigning one inside the loop moves the original, and the loop variable is ONE variable reassigned per iteration, exactly like a handwritten `while` (see [functions, closures, and generics](functions.md)). Accumulating through a shared `vec.push(v)` works, and so does plain reassignment.":
  "`for (let v of it)` 会脱糖成 `it.iterate(emit)` 加一个合成闭包，而捕获法则像对待每个闭包一样对待它：标量按复制走（循环内的重绑定只留在局部），以引用为头的绑定共享槽位——在循环内重新赋值会移动原绑定，且循环变量是每一轮被重新赋值的同一个变量，与手写的 `while` 完全一致（参见[函数、闭包与泛型](functions.md)）。通过共享的 `vec.push(v)` 累积可行，直接重新赋值同样可行。",

 "**Loop variables share elements; the binding is one variable.** A `for (let x of xs)` loop hands you the stored element; writes through it mutate the sequence. The loop variable is ONE binding reassigned per iteration — `for..of` is sugar for a `while`\\-shaped loop, and the capture law treats it like any other ref-headed binding.":
  "**循环变量共享元素；绑定是同一个变量。** `for (let x of xs)` 循环把存储的元素交给你；穿过它的写入会改动序列本身。循环变量是每一轮被重新赋值的同一个绑定——`for..of` 是 `while`\\-形循环的语法糖，捕获法则像对待其他以引用为头的绑定一样对待它。",

 "`for (let v of it) { body }` desugars to `it.iterate(emit)` with a synthetic closure: the body runs, then `emit` returns `true`; `break` returns `false`. The loop variable is ONE variable reassigned per iteration — the same law the fused index loops follow (the capture law treats the two forms identically; see [functions, closures, and generics](../reference/functions-closures-generics.md)). The builtin sequences (`[T]`, `str`, `bytes`, and the standard growable `Vec`) keep fused index loops instead; they never pay a per-element call.":
  "`for (let v of it) { body }` 会脱糖成 `it.iterate(emit)` 加一个合成闭包：先跑循环体，然后 `emit` 返回 `true`；`break` 返回 `false`。循环变量是每一轮被重新赋值的同一个变量——与融合索引循环节点遵循的是同一条法则（捕获法则对两种形式一视同仁；参见[函数、闭包与泛型](../reference/functions-closures-generics.md)）。内建序列（`[T]`、`str`、`bytes` 以及标准的可增长 `Vec`）保留融合索引循环；它们从不支付逐元素调用的开销。",

 "Iterating form — vecs, fixed arrays, slices, strings (one-codepoint `str`s per step), `bytes` (`u8` per step), and any type with a registered `Iterator` impl — an enum value included (see [Traits and dispatch](traits.md) and [Enums](enums.md)). Both spellings of this form are one loop: the loop variable is a single binding reassigned per iteration, and over a user iterable the loop desugars to an `it.iterate(emit)` closure that follows the capture law exactly like the fused form:":
  "迭代形式——vec、定长数组、切片、字符串（每步一个码点的 `str`）、`bytes`（每步一个 `u8`），以及任何注册了 `Iterator` impl 的类型——枚举值也在内（参见[trait 与分派](traits.md)与[枚举](enums.md)）。这一形式的两种写法是同一个循环：循环变量是每一轮被重新赋值的单个绑定，而在用户可迭代对象之上，循环会脱糖为 `it.iterate(emit)` 闭包，与融合形式完全一致地遵循捕获法则：",

 "**The capture law**: primitives, `nil`, and `fn` values copy — a captured scalar is the closure's own slot, and reassigning it inside the closure never moves the original. Ref-headed values (records, `Vec`, arrays, `str`/`bytes`, `?T`, enums) cross as handles: writes through the captured handle reach the shared cell, and so does a whole-value REASSIGNMENT — a captured-and-reassigned binding is promoted to a hidden shared-slot cell, so both frames stay linked for the binding's whole scope (a stash-then-reassign, a reassign inside the closure, and the `for..of` loop variable all follow this one law). Refcounting keeps captures alive; a closure is itself a shared cell value.":
  "**捕获法则**：标量、`nil` 和 `fn` 值按复制走——被捕获的标量是闭包自己的槽位，在闭包内重新赋值绝不会移动原绑定。以引用为头的值（记录、`Vec`、数组、`str`/`bytes`、`?T`、枚举）以句柄跨越：穿过被捕获句柄的写入抵达共享单元，整体重新赋值也是如此——一个既被捕获又被重新赋值的绑定会晋升为隐藏的共享槽位单元，两个帧在整个绑定作用域内保持联动（先暂存再重赋值、闭包内重赋值、`for..of` 循环变量，都遵循这同一条法则）。引用计数让捕获保持存活；闭包本身就是一个共享单元值。",

 # ---- declared impl generics ----

 "`impl Trait for Type { .. }` registers the pair. Every method must match the trait's signature exactly; extra methods don't belong here (put those in an inherent block). Trait impl methods carry no `pub` — they are as visible as the trait. Generic binders are **declared after `impl`** — `impl<T> Readable<T> for Source<T>` — and the declared list is the definition site: a bare parameter name in the head that it does not declare is an error naming the fix.":
  "`impl Trait for Type { .. }` 注册这一对。每个方法都必须与 trait 的签名完全一致；多余的方法不属于这里（把它们放进 inherent 块）。trait impl 方法不带 `pub`——它们的可见性与 trait 一致。泛型绑定器**在 `impl` 之后声明**——`impl<T> Readable<T> for Source<T>`——且这份声明列表就是定义位置：头部出现的裸参数名若不在其中，就是一个会指明修复方式的错误。",

 "`impl Type { .. }` — no trait — is where a type's own methods live: class methods like `new`, instance methods, helpers. It compiles only in the module that declares the type. A generic type's inherent impl declares its binders the same way a trait impl does: `impl<T> Vec<T> { .. }`.":
  "`impl Type { .. }`——不带 trait——是一个类型自有方法的归宿：`new` 这样的类方法、实例方法、辅助方法。它只在声明该类型的模块里编译。泛型类型的 inherent impl 以与 trait impl 相同的方式声明它的绑定器：`impl<T> Vec<T> { .. }`。",

 "**Generic traits and generic targets**: `trait Wrap<T>` gives each type-argument list its own instantiation (`Wrap<i32>` ≠ `Wrap<str>`). Generic binders are **declared after `impl`** — `impl<A, B> Hashable for Pair<A, B>` — and that list is the definition site: a bare parameter name in the head is a use that must resolve against it (an undeclared name is the error it always should have been — \"undeclared type parameter `T` — declare it: `impl<T> ..`\"). The same law covers inherent impls: `impl<T> Vec<T> { .. }`, not `impl Vec<T> { .. }`. Parameterized trait impls are legal: `impl<T> Readable<T> for Source<T>` registers a **template** serving every concrete instantiation; a hand-written concrete impl shadows the template; repeated parameters (`impl<T> W<T, T> for Pair2<T>`) are legal. Each trait argument must be a concrete type or a declared binder that names one of the target's own parameters.":
  "**泛型 trait 与泛型目标**：`trait Wrap<T>` 为每个类型实参列表给出独立的实例化（`Wrap<i32>` ≠ `Wrap<str>`）。泛型绑定器**在 `impl` 之后声明**——`impl<A, B> Hashable for Pair<A, B>`——且这份列表就是定义位置：头部的裸参数名是一次使用，必须能对它解析（未声明的名字终于成为它本该是的错误——\"undeclared type parameter `T` — declare it: `impl<T> ..`\"）。同一条法则也覆盖 inherent impl：`impl<T> Vec<T> { .. }`，而不是 `impl Vec<T> { .. }`。带参数的 trait impl 是合法的：`impl<T> Readable<T> for Source<T>` 注册一个服务每个具体实例化的**模板**；手写的具体 impl 会遮蔽模板；重复的参数（`impl<T> W<T, T> for Pair2<T>`）也合法。每个 trait 实参必须是具体类型，或是命名目标自身某个参数的已声明绑定器。",
 # ---- the flow package (push pipeline) ----

 "`flow` — the push pipeline":
  "`flow` —— 推送管线",

 "`Flow<E>` chains the push contract. A type is iterable when it registers `impl Iterable<E> for T` — `for (x of it)` desugars to `it.iterate(emit)` — and a Flow wraps one drive in adapter stages: a closure per stage, never per element. Entry is `into_flow()`, the exit is a sink (`Vec.from_flow`, a fixed array), and everything between is chaining:":
  "`Flow<E>` 把推送契约串成链。一个类型在注册了 `impl Iterable<E> for T` 时即可迭代——`for (x of it)` 脱糖为 `it.iterate(emit)`——而 Flow 把同一次 drive 包进适配器阶段：每个阶段一个闭包，绝非每个元素一个。入口是 `into_flow()`，出口是汇（`Vec.from_flow`、定长数组），中间全是链式拼接：",

 "Three laws to know:":
  "三条要知道的法则：",

 "**`map` introduces a new type variable.** Annotate the lambda, spell the type argument (`.map<i32>(..)`), or pass a fn path — a lambda that spells nothing diagnoses with the fix.":
  "**`map` 引入新的类型变量。** 给 lambda 标注类型、写明类型实参（`.map<i32>(..)`），或传入具名 fn——什么都不标注的 lambda 会得到指明修复方式的诊断。",

 "**Stateful stages are single-shot.** `take`/`skip` hold their counter in a record the drive mutates; a drained stage stays drained. `take` answers `false` at its stop, which stops the SOURCE — the elements after it are never driven.":
  "**有状态的阶段是一次性的。** `take`/`skip` 把计数器放在一个记录里由 drive 改动；耗尽的阶段保持耗尽。`take` 在停止点回答 `false`，这会停下源——其后的元素永不被驱动。",

 "Pipelines are the clarity tier: each stage costs one indirect call per element, and the fused builtin loops stay the perf tier. The full member table lives in [the standard library reference](../reference/stdlib.md#flow-the-push-pipeline).":
  "管线是清晰度档位：每个阶段对每个元素付出一次间接调用，融合的内建循环仍是性能档位。完整成员表见[标准库参考](../reference/stdlib.md#flow-the-push-pipeline)。",

 "`Flow<E>` chains the push contract: a type is iterable when it registers `impl Iterable<E> for T`, and a Flow wraps one drive in adapter stages — a closure per STAGE, never per element. Pipelines are the clarity tier; the fused builtin loops stay the perf tier.":
  "`Flow<E>` 把推送契约串成链：一个类型在注册了 `impl Iterable<E> for T` 时即可迭代，而 Flow 把同一次 drive 包进适配器阶段——每个阶段一个闭包，绝非每个元素一个。管线是清晰度档位；融合的内建循环仍是性能档位。",

 "`x.into_flow() -> Flow<E>`":
  "`x.into_flow() -> Flow<E>`",

 "the entry — `IntoFlow<E>` impls: `[T]`, `str`, `bytes`, `Vec<T>`, and `Flow<E>` itself (the identity row: a chain re-enters as a source)":
  "入口——`IntoFlow<E>` 行：`[T]`、`str`、`bytes`、`Vec<T>`，以及 `Flow<E>` 自身（同一性行：一条链重新作为源进入）",

 "`map<R>(f: fn(E) -> R) -> Flow<R>`":
  "`map<R>(f: fn(E) -> R) -> Flow<R>`",

 "transform — introduces a new type variable, so annotate the lambda or spell the type argument; a fn path infers":
  "变换——引入新的类型变量，因此请标注 lambda 或写明类型实参；具名 fn 可自动推断",

 "`filter(p: fn(E) -> bool) -> Self`":
  "`filter(p: fn(E) -> bool) -> Self`",

 "keep the elements the predicate admits":
  "保留谓词放行的元素",

 "`take(n)` / `skip(n)`":
  "`take(n)` / `skip(n)`",

 "the first `n` / everything after the first `n` — the counter is a record the drive mutates, so a drained stage stays drained":
  "前 `n` 个 / 跳过前 `n` 个之后的全部——计数器是一个由 drive 改动的记录，耗尽的阶段保持耗尽",

 "`count() -> i32` / `for_each(f)` / `fold<R>(init, f) -> R` / `enumerate() -> Flow<(i32, E)>`":
  "`count() -> i32` / `for_each(f)` / `fold<R>(init, f) -> R` / `enumerate() -> Flow<(i32, E)>`",

 "the one-drive consumers":
  "单次 drive 的消费者",

 "`T::from_flow(it: Iterable<E>) -> T`":
  "`T::from_flow(it: Iterable<E>) -> T`",

 "the sink — `FromFlow<E>` impls: `Vec<T>`, `HashSet<T>`, and the fixed `[E]` (empty is `[]`; non-empty seeds from element 0, so the source must re-drive)":
  "汇——`FromFlow<E>` 行：`Vec<T>`、`HashSet<T>`，以及定长 `[E]`（空即 `[]`；非空以元素 0 作种子，因此源必须可重新驱动）",

 "`for (x of chain)`":
  "`for (x of chain)`",

 "chains are iterables — the `impl Iterable<E> for Flow<E>` row feeds the ordinary desugar":
  "链即可迭代对象——`impl Iterable<E> for Flow<E>` 行进入普通的脱糖",

 "**Import the traits you spell and the pipeline type**: `use flow::{ Flow, IntoFlow, FromFlow }` — the use-both law names each one; the sink's `Iterable<E>` parameter re-mints through `use core::{ Iterable }` (the `from_flow` sinks take the CONTRACT, not Flow, so a user iterable — a `CountUp`\\-shape — widens straight into them).":
  "**拼写了什么就导入什么，连同管线类型**：`use flow::{ Flow, IntoFlow, FromFlow }`——使用即导入法则逐一点名；汇的 `Iterable<E>` 参数经 `use core::{ Iterable }` 重铸（`from_flow` 汇取的是契约而非 Flow，因此用户可迭代对象——`CountUp`\\ 形状——直接加宽进入）。",

 "**The entries and sinks are traits** — `IntoFlow<E>` rows exist for the builtin sequences (`[T]`, `str`, `bytes`), for `Vec<T>`, and for `Flow<E>` itself (the identity row: a chain re-enters as a source), so generic code bounded on `IntoFlow<E>` takes chains and sources alike. The sinks (`FromFlow<E>`) take the CONTRACT — `it: Iterable<E>` — so a user iterable (a `CountUp`\\-shape, no Flow involved) widens straight into them.":
  "**延后但有记载**：mapset 的 `IntoFlow`（nmapset 尚无宿主表遍历——json 的 map 编码将与它一同成长）和 `HashMap::from_flow`（其 `(K, V)` 元组 trait 实参尚无法跨越编译包的 impl 行载体；它在 flow 自己的单元内编译并分派）。",

}


def main():
    pot_text = open(POT, encoding='utf-8').read()
    po_text = open(PO, encoding='utf-8').read()
    pot_header, pot_entries = parse(pot_text)
    po_header, po_entries = parse(po_text)
    old = dict((m, s) for m, s, _ in po_entries)
    old_ids = set(old)

    header = po_header if po_header is not None else pot_header or ''
    # the header rides EXACTLY as the old file wrote it: one long
    # msgstr line, real newlines escaped (gettext metadata needs the
    # trailing \n per line; this reproduces the original bytes)
    out = [f'msgid ""\nmsgstr "{esc(header)}"\n']

    kept = carried = fresh = fallback = 0
    unused_fresh = set(FRESH)

    for msgid, _, refs in pot_entries:
        msgstr = None
        if msgid in old:
            msgstr = old[msgid]
            if msgstr:
                kept += 1
            elif msgid in FRESH:
                msgstr = FRESH[msgid]
                unused_fresh.discard(msgid)
                fresh += 1
            else:
                fallback += 1
        else:
            heir = next((o for o in old_ids if o != msgid and mech(o) == msgid), None)
            if heir is not None and old[heir]:
                msgstr = mech(old[heir])
                carried += 1
            elif msgid in FRESH:
                msgstr = FRESH[msgid]
                unused_fresh.discard(msgid)
                fresh += 1
            else:
                msgstr = ''
                fallback += 1
        q = quote(msgid)
        entry = refs + 'msgid ' + '\n'.join(q) + '\n'
        qs = quote(msgstr)
        entry += 'msgstr ' + '\n'.join(qs) + '\n\n'
        out.append(entry)

    open(PO, 'w', encoding='utf-8').write('\n'.join(out))
    obsolete = old_ids - {m for m, _, _ in pot_entries}
    print(f'fold: {len(pot_entries)} entries — kept {kept}, mech-carried {carried}, '
          f'fresh {fresh}, fallback-to-en {fallback}; obsolete dropped {len(obsolete)}')
    if unused_fresh:
        print(f'WARNING: {len(unused_fresh)} FRESH entries unused:')
        for u in list(unused_fresh)[:5]:
            print('  !', u[:100])
        sys.exit(1)


if __name__ == '__main__':
    main()
