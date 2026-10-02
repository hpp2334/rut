#!/usr/bin/env python3
"""The po fold — merge the fresh rut.pot into docs/po/zh_CN.po.

Merge law (the GLOSSARY contract, after the 432ace9 lesson):
- entries in POT order; the header rides from the existing zh_CN.po;
- an UNCHANGED msgid keeps its msgstr byte-for-byte;
- a CHANGED msgid whose diff is exactly the mechanical cutover
  (rut.json → rut.jsonc, wire v7→v9 / v8→v10, the main convention's
  `pub fn main` → `entry fn main`) carries the old msgstr
  through the SAME mechanical substitution — the zh text moves with
  the file name, the wire numbers, and the decl spelling, nothing else;
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

# Fresh translations for the strings this sweep changed (the book
# stops negating names it never had — pure-positive rewrites; the
# negating clauses are gone, so each fresh msgstr is the old zh with
# the foreign-name negation dropped). Terminology per
# docs/po/GLOSSARY.md; code spans byte-identical; the extracted
# markup style is mirrored (`_em_` emphasis, `**bold**`). Everything
# not in FRESH either keeps its msgstr byte-for-byte (unchanged
# msgid), rides MECH (the mechanical cutover shapes), or folds to ""
# — the English fallback BY DESIGN.
FRESH = {
    # ---- lanes 3+4 (the select + type-surface plan): `Future<T>` the
    # closed builtin class + the `async { }` block primitive, and the
    # bracket markers replacing the `builtin trait` rows. Terminology per
    # GLOSSARY: bracket marker → 括号标记, designated slot → 指定槽位,
    # mint → 铸造, async block → async 块, inject → 注入.
    "Engine contracts — markers and the closed async pair":
        "引擎契约——标记与封闭的 async 对",

    "Engine contracts — markers and closed classes":
        "引擎契约——标记与封闭类",

    "Engine contracts are not traits":
        "引擎契约不是 trait",

    "Engine contracts — the `[iterable]` marker":
        "引擎契约——`[iterable]` 标记",

    "The iteration protocol — the `[iterable]` marker":
        "迭代协议——`[iterable]` 标记",

    "Disposal — the cell-death contract (the `[disposal]` marker)":
        "Disposal——单元死亡契约（`[disposal]` 标记）",

    "Destruction — the `[disposal]` marker":
        "销毁——`[disposal]` 标记",

    "The producers — `async { }` blocks and the injected cx":
        "生产者——`async { }` 块与注入的 cx",

    "The async block":
        "async 块",

    "`async fn`, `async { }`, and `await`":
        "`async fn`、`async { }` 与 `await`",

    "`Future<T>` — the one async type":
        "`Future<T>`——唯一的 async 类型",

    "contract":
        "契约",

    "`[iterable]`":
        "`[iterable]`",

    "`[disposal]`":
        "`[disposal]`",

    "`disposal` / `iterable`":
        "`disposal` / `iterable`",

    "`pub builtin class` — closed":
        "`pub builtin class` —— 封闭",

    "`pub builtin class` — closed; `checkpoint() -> u32`, `cancelled() -> bool`":
        "`pub builtin class` —— 封闭；`checkpoint() -> u32`、`cancelled() -> bool`",

    "`fn <free>(self, emit: fn(E) -> bool)` on an inherent impl":
        "固有 impl 上的 `fn <自由名>(self, emit: fn(E) -> bool)`",

    "`fn <free>(mut self, cx: DisposalContext)` on an inherent impl":
        "固有 impl 上的 `fn <自由名>(mut self, cx: DisposalContext)`",

    "`T::from_flow(it: Flow<E>) -> T`":
        "`T::from_flow(it: Flow<E>) -> T`",

    "the bracket markers — `[disposal] fn` / `[iterable] fn` designate an inherent impl member as an engine contract slot (before visibility: `[disposal] pub fn ..`); the set is closed and engine-owned ([traits](traits.md))":
        "括号标记——`[disposal] fn` / `[iterable] fn` 把固有 impl 成员指定为引擎契约槽位（在可见性之前：`[disposal] pub fn ..`）；标记集是封闭的、由引擎拥有（[traits](traits.md)）",

    "`async { .. }` — keyword before a block — is the async primitive: one expression form whose evaluation mints the frame (pure; legal in sync code) and whose type is `Future<T>` ([async and await](async.md)). `async` before `fn` is the declaration sugar over it.":
        "`async { .. }` —— 关键字后接块——就是 async 原语：一种表达式形式，求值即铸造帧（纯操作；同步代码中合法），其类型是 `Future<T>`（[async and await](async.md)）。`fn` 前的 `async` 是它之上的声明糖。",

    "The engine's own contracts are **spellings on types**, not traits (the `builtin trait` row kind is gone). Two mechanisms, one job each:":
        "引擎自己的契约是**类型上的拼写**，不是 trait（`builtin trait` 行类别已移除）。两种机制，各司一职：",

    "**The bracket markers** — `[disposal]` and `[iterable]` designate an inherent impl member. The parser accepts any contextual word in the brackets; the checker validates the engine's CLOSED set, at most one member per contract per class, inherent-members-only, and each contract's signature. The descriptor ABI is a designated slot per class — the engine's release path and the for-of weave read their slot directly, never a per-(type × trait) lookup:":
        "**括号标记** —— `[disposal]` 与 `[iterable]` 指定一个固有 impl 成员。解析器接受方括号里的任何上下文词；检查器校验引擎的**封闭**标记集：每个类每个契约至多一个成员、仅限固有成员、且签名必须符合契约。描述符 ABI 是每类一个指定槽位——引擎的释放路径与 for-of 编织直接读取自己的槽位，绝不走每（类型 × trait）查找：",

    "`[disposal]`'s contract: `fn <free>(mut self, cx: DisposalContext)` — the engine calls it when a value of the type reaches refcount zero ([the Rc heap](rc-heap.md)); the target must be a CONCRETE struct or class (the row keys the cell's type id). `[iterable]`'s contract is the iteration protocol above.":
        "`[disposal]` 的契约：`fn <自由名>(mut self, cx: DisposalContext)`——当该类型的值的引用计数归零时，引擎调用它（[the Rc heap](rc-heap.md)）；目标必须是具体的结构体或类（行以单元的类型 id 为键）。`[iterable]` 的契约即上文迭代协议。",

    "**The closed builtin classes** — `Future<T>` and `RunContext` carry the async protocol. Both are `pub builtin` (the import-gated spelling) and **closed**: no constructor, no impl lane, no user-callable members beyond `RunContext`'s two reads — a user type cannot BE a future or a cx. The walls hold by nominal closure: structural satisfaction cannot see engine identity, so closure is the only spelling of \"engine-minted only\". The ENGINE weaves without them — async frames, the minted cx, and the fused `for..of` loops never consult user scope; source that SPELLS a name resolves it only through `use core::{ .. }` (a launcher's `f: Future<T>`, `downcast<Future<..>>`, a `cx` probe in an async body — see [Async and await](async.md), [host fns and declaration files](host-fns.md), and [the Rc heap](rc-heap.md)).":
        "**封闭的 builtin 类** —— `Future<T>` 与 `RunContext` 承载 async 协议。二者都是 `pub builtin`（导入门控拼写）且是**封闭的**：没有构造器、没有 impl 通道、除 `RunContext` 的两个读操作外没有用户可调成员——用户类型不可能成为一个 future 或一个 cx。墙靠名义封闭而立：结构化满足看不见引擎身份，因此封闭是\"仅引擎铸造\"的唯一拼写。**引擎**在没有它们的情况下编织——async 帧、铸造的 cx、融合的 `for..of` 循环从不查询用户作用域；拼写了名字的源码只能通过 `use core::{ .. }` 解析（启动器的 `f: Future<T>`、`downcast<Future<..>>`、async 体内的一次 `cx` 探测——见 [Async and await](async.md)、[host fns and declaration files](host-fns.md) 与 [the Rc heap](rc-heap.md)）。",

    "The engine's own contracts are spellings on types (the `builtin trait` row kind is gone):":
        "引擎自己的契约是类型上的拼写（`builtin trait` 行类别已移除）：",

    "rut's concurrency is **pull-based**. An `async fn` compiles into the driven half of a hidden frame type — a **cold future** that only progresses when driven. There are no promises, no microtask queue, and no implicit scheduling: **the host owns time**. One noun (`Future<T>`, a closed builtin class), one consume law (`await` **or** launch — exactly one), one pender (`sleep`; an embedder may mount its own).":
        "rut 的并发是**拉取式**的。`async fn` 编译成隐藏帧类型的被驱动半——一个只有被驱动才会前进的**冷 future**。没有 promise、没有微任务队列、没有隐式调度：**时间由宿主拥有**。一个名词（`Future<T>`，封闭的 builtin 类），一条消费法则（`await` **或**启动——恰好一个），一个 pender（`sleep`；嵌入方可挂载自己的）。",

    "Every async producer answers the **same** type: an `async fn` call, an `async { }` block, `sleep`, `select2`, `completer` — all `Future<..>`, so `await`/`select2`/`launch_future` never ask \"which kind\". `Future<T>` is a **closed** `builtin class`: no constructor, no impl lane, no user-callable members — a user type cannot BE a future (structural satisfaction cannot see engine identity, so nominal closure is the only spelling of \"engine-minted only\"). The hidden frame is representation; the handle is the surface.":
        "每个 async 生产者的回答都是**同一个**类型：`async fn` 调用、`async { }` 块、`sleep`、`select2`、`completer`——全是 `Future<..>`，因此 `await`/`select2`/`launch_future` 从不问\"哪一种\"。`Future<T>` 是一个**封闭的** `builtin class`：没有构造器、没有 impl 通道、没有用户可调成员——用户类型不可能成为一个 future（结构化满足看不见引擎身份，因此名义封闭是\"仅引擎铸造\"的唯一拼写）。隐藏帧是表示；句柄是表面。",

    "`async fn` is genuine **sugar** — both sides of the law are writable programs:":
        "`async fn` 是真正的**糖**——法则的两边都是可写的程序：",

    "The `async { }` **block** is the primitive: one expression form (keyword before a block). Evaluating it **mints the frame** — the mint is pure, so an async block is legal in sync code (`launch_future(async { .. })`, a `select2` arm); the frame runs when driven. Its type is `Future<T>` with `T` inferred from the block's `return` statements (fn-body rules); an annotated position pins it (`let f: Future<str> = async { .. }`). The block's reads capture the enclosing locals **by value** at the mint (the capture law: scalars copy, ref-headed handles share their cell), and a `return` inside the block answers **the future** — not any enclosing fn.":
        "`async { }` **块**是原语：一种表达式形式（关键字后接块）。对它的求值**铸造帧**——铸造是纯操作，因此 async 块在同步代码中合法（`launch_future(async { .. })`、一个 `select2` 臂）；帧在被驱动时运行。其类型是 `Future<T>`，`T` 由块的 `return` 语句推断（fn 体法则）；带标注的位置会把它钉死（`let f: Future<str> = async { .. }`）。块的读取在铸造时**按值**捕获外围局部（捕获法则：标量复制，引用头的句柄共享其单元），块内的 `return` 回答的是**这个 future**——不是任何外层 fn。",

    "**The cx is injected, never spelled.** The weave binds `cx` (a `RunContext`) in every async body; an async fn's parameters are ordinary values — a spelled `cx: RunContext` diagnoses.":
        "**cx 是注入的，绝不拼写。** 编织在每个 async 体内绑定 `cx`（一个 `RunContext`）；async fn 的参数是普通值——拼写出 `cx: RunContext` 会被诊断。",

    "`await` is legal only inside an async body (an `async fn` or an `async { }` block), and its operand must be a `Future<..>` — plain type identity, since every producer mints the closed class.":
        "`await` 只在 async 体内合法（`async fn` 或 `async { }` 块），且其操作数必须是 `Future<..>`——纯类型同一性，因为每个生产者铸造的都是封闭类。",

    "Async **methods** are not woven in this build — async points are free fns or `async { }` blocks inside sync methods (the standard HTTP face mints inside its methods).":
        "此构建不编织 async **方法**——async 点是自由 fn，或同步方法内的 `async { }` 块（标准 HTTP 门面就在自己的方法内铸造）。",

    "Cancellation-by-drop: the probe at the resumed checkpoint branches to a drop path that releases the frame's cell-backed locals (their `[disposal]` members run, [the Rc heap](rc-heap.md)), retires the state, and returns.":
        "以 drop 取消：恢复检查点处的探测分支到一条 drop 路径，释放帧的单元支撑局部（它们的 `[disposal]` 成员会运行，[the Rc heap](rc-heap.md)），使状态退役，然后返回。",

    "`RunContext` — a closed `builtin class`, injected by the weave — carries two reads, usable as plain data inside any async body:":
        "`RunContext`——一个封闭的 `builtin class`，由编织注入——承载两个读操作，可在任何 async 体内当作普通数据使用：",

    "Cancellation is data the frame reads — never an exception it catches. (The weave's own `next_checkpoint` edge is unspelled: the weave records each await's resume state.)":
        "取消是帧读取的数据——绝不是它捕获的异常。（编织自己的 `next_checkpoint` 边不可拼写：每个 await 的恢复状态由编织记录。）",

    "Launchers are **host surface** — ordinary rut code over the closed `Future` class, provided by the `rut/async_host` package (users may write their own the same way):":
        "启动器是**宿主面**——封闭 `Future` 类之上的普通 rut 代码，由 `rut/async_host` 包提供（用户可以同样方式编写自己的启动器）：",

    "rut's concurrency is **pull-based**, built on one noun: `Future<T>` — a closed builtin class, engine-minted only. Calling an `async fn` (or evaluating an `async { }` block) runs nothing — it produces a cold future. The future runs when something *drives* it: an `await` inside another async body, or a launch into the VM's queue.":
        "rut 的并发是**拉取式**的，建立在一个名词上：`Future<T>`——封闭的 builtin 类，仅由引擎铸造。调用 `async fn`（或求值一个 `async { }` 块）什么都不运行——它产生一个冷 future。future 在某个东西**驱动**它时才运行：另一个 async 体内的一次 `await`，或一次进入 VM 队列的启动。",

    "**The context is injected, never spelled.** The weave binds `cx` (a `RunContext`) in every async body; an async fn's parameters are ordinary values. It carries the frame edge: `checkpoint()` reads the resume state, `cancelled()` reads the frame's abort flag. The `RunContext` class is core's, import-gated (`use core::{ RunContext }`) — the weave itself never needs the import, only source that names the class does.":
        "**上下文是注入的，绝不拼写。** 编织在每个 async 体内绑定 `cx`（一个 `RunContext`）；async fn 的参数是普通值。它承载帧边：`checkpoint()` 读恢复状态，`cancelled()` 读帧的取消标志。`RunContext` 类属于 core，导入门控（`use core::{ RunContext }`）——编织本身从不需要导入，只有点名这个类的源码才需要。",

    "**Every producer has one type.** An async fn call, an async block, `sleep`, the race/completer mints — all `Future<..>`, so `await` never asks \"which kind\". The walls hold by nominal closure: no constructor, no impl lane — a user type cannot BE a future (which is how the standard `sleep` itself is minted: engine-side, sealed under the class spelling).":
        "**每个生产者只有一个类型。** async fn 调用、async 块、`sleep`、竞速/补全器的铸造——全是 `Future<..>`，因此 `await` 从不问\"哪一种\"。墙靠名义封闭而立：没有构造器、没有 impl 通道——用户类型不可能成为一个 future（标准的 `sleep` 自己也是这样铸造的：引擎侧，以类的拼写密封）。",

    "A type is iterable when an inherent impl marks a member `[iterable]` (the bracket marker — the contract's shape is `fn <free>(self, emit: fn(E) -> bool)`, the element falling out of the marked member's own signature):":
        "当一个固有 impl 把某个成员标记为 `[iterable]` 时，该类型就是可迭代的（括号标记——契约的形状是 `fn <自由名>(self, emit: fn(E) -> bool)`，元素类型从被标记成员自己的签名中掉出来）：",

    "The contract's shape: `fn <free>(self, emit: fn(E) -> bool)` — the member's NAME IS FREE (the bracket designates, never the spelling), and the element type `E` falls out of the marked member's own emit parameter. `for (v of it) { body }` calls that ONE designated member — `it.<member>(emit)` — with a synthetic closure: the body runs, then `emit` returns `true`; `break` returns `false` (stopping the iteration); `continue` returns `true` immediately; a `return` inside the body stops the iteration (not the enclosing function). The loop variable is the closure's parameter — a fresh binding per iteration; captured enclosing locals are copied by value at the desugar, so accumulate through a shared cell or a method. The builtin sequences (`[T]`, `Vec<T>`, `str`, `bytes`) keep their fused index loops and never reach the marker. An enum value iterates the same way — a marked member on the enum's impl makes `for (let v of c)` walk whatever it emits (see [Enums](enums.md)).":
        "契约的形状：`fn <自由名>(self, emit: fn(E) -> bool)`——成员的名字是**自由的**（以方括号指定，绝不是以拼写指定），元素类型 `E` 从被标记成员自己的 emit 参数中掉出来。`for (v of it) { body }` 调用那**一个**指定成员——`it.<成员>(emit)`——带着一个合成闭包：体先运行，然后 `emit` 返回 `true`；`break` 返回 `false`（停止迭代）；`continue` 立即返回 `true`；体内的 `return` 停止的是迭代（不是外层函数）。循环变量是闭包的参数——每次迭代一个新绑定；被捕获的外围局部在脱糖时按值复制，所以要经共享单元或方法来累加。内置序列（`[T]`、`Vec<T>`、`str`、`bytes`）保留融合索引循环，从不触及该标记。枚举值以同样的方式迭代——枚举 impl 上的一个被标记成员让 `for (let v of c)` 走遍它发出的任何东西（见 [Enums](enums.md)）。",

    "Destructors are implemented, not attached. A type opts in with one `[disposal]`-marked member on its inherent impl (the name is free — the bracket designates):":
        "析构器是被实现的，不是被附着的。一个类型通过在其固有 impl 上标记一个 `[disposal]` 成员来选择加入（名字是自由的——以方括号指定）：",

    "A `[disposal]` member runs when the cell's refcount reaches **zero** — deterministic destruction, not a collector callback. The dying value arrives as `mut self`; the fields release after the body returns. The member's NAME IS FREE — the bracket designates, the spelling never does (one `[disposal]` member per class).":
        "`[disposal]` 成员在单元的引用计数到达**零**时运行——确定性的销毁，不是收集器回调。垂死的值以 `mut self` 到达；字段在体返回后释放。成员的名字是**自由的**——以方括号指定，绝不以拼写指定（每个类一个 `[disposal]` 成员）。",

    "`cx: DisposalContext` is **engine-minted**, one per call. It is empty today — the parameter exists so the context can grow additively without ever touching the marker's signature.":
        "`cx: DisposalContext` 是**引擎铸造的**，每次调用一个。它今天还是空的——这个参数的存在，是为了上下文可以增量增长而不必触碰标记的签名。",

    "`DisposalContext` is the **import-gated** builtin class: `use core::{ DisposalContext };` brings it in ([core and the swappable packages](stdlib.md)). Using it without the use line diagnoses `` `DisposalContext` is not in scope — `use core::{ DisposalContext }` ``. The `Disposal` trait is GONE — the `[disposal]` marker needs no import, and the removed spelling diagnoses with the marker's shape.":
        "`DisposalContext` 是**导入门控**的 builtin 类：`use core::{ DisposalContext };` 把它带进来（[core and the swappable packages](stdlib.md)）。没有 use 行就使用它会诊断 `` `DisposalContext` is not in scope — `use core::{ DisposalContext }` ``。`Disposal` trait 已移除——`[disposal]` 标记不需要导入，被移除的拼写会以标记的形状给出诊断。",

    "One marked member per class, by the marker's own law — there is no per-value attach and nothing to attach twice.":
        "每个类一个被标记成员，这是标记自己的法则——没有逐值的附着，也没有可以附着两次的东西。",
    # ---- the newtype landing (Lane 2 of the select + type-surface
    # plan): classes.md's "Newtypes: the one-field wrapper" section and
    # the lexical-structure construction-bullet amendment. Terminology
    # per GLOSSARY: newtype → 新类型, wrapper → 包装器, binder → 绑定名.
    "Newtypes: the one-field wrapper":
        "新类型：单字段包装器",

    "`class Name(Wrapped);` — the positional one-field decl. It IS an ordinary class: exactly one field, `inner`, of the wrapped type. The positional spelling additionally provides the constructor — the call form `Name(value)`:":
        "`class Name(Wrapped);` —— 位置式单字段声明。它就是一个普通的类：恰好一个字段 `inner`，类型是被包装的类型。位置式写法额外提供了构造器——调用形式 `Name(value)`：",

    "**The wrapped type is unrestricted** — primitives, composites, tuples, fn types, another module's class: `class Opt(?i64);`, `class Pair((i32, str));`, `class Cb(fn(i32) -> str);` are all legal. The wrapped position is a FIELD type, never an impl target — that is the point. A wrapper is how a primitive or a third-party type gains a capability satisfaction can't reach: declare the wrapper, implement on it, hand the wrapper across the boundary.":
        "**被包装的类型不受限制** —— 原始类型、复合类型、元组、fn 类型、另一个模块的类：`class Opt(?i64);`、`class Pair((i32, str));`、`class Cb(fn(i32) -> str);` 全都合法。被包装的位置是一个字段类型，绝不是 impl 目标——这正是要点。包装器，就是让原始类型或第三方类型获得满足（satisfaction）够不着的能力的方式：声明包装器，在其上实现，把包装器递过边界。",

    "**No auto-insertion, ever.** `dump(64)` is an error forever; the capability exists exactly where the constructor is spelled. And no implicit forwarding: the wrapper exposes exactly its own members — `inner` (the ordinary field, ordinary visibility) and its `impl` methods.":
        "**绝不自动插入。** `dump(64)` 永远是错误；能力只存在于拼写出构造器的地方。也没有隐式转发：包装器暴露的恰好是它自己的成员——`inner`（普通字段，普通可见性）与它的 `impl` 方法。",

    "**The naming law**: wrappers are named for what they wrap (`JsonI64`, never `JsonExt`). Per-type wrappers with type-specific bodies when the bodies differ; a generic wrapper covers the uniform cases: `class DebugWrap<T>(T);`.":
        "**命名法则**：包装器以它所包装的东西命名（`JsonI64`，绝不是 `JsonExt`）。方法体各异时用逐类型的包装器，各带自己的方法体；一致的地方用泛型包装器覆盖：`class DebugWrap<T>(T);`。",

    "**The generic instantiation law**: a binder is determined by the wrapped argument when it appears there (`class Tail<T>([T]);` → `Tail([1, 2])` is a `Tail<i32>`), or by explicit type arguments at the constructor (`Converter<i32>(\"64\")` — `class Converter<T>(str);` cannot infer, the binder is not in `str`). If any binder is undetermined, the call spells ALL binders — all-or-nothing: `class Pair<A, B>(i32);` accepts `Pair<i32, str>(7)`, never `Pair<i32>(7)`.":
        "**泛型实例化法则**：绑定名出现在被包装实参中时，由该实参确定（`class Tail<T>([T]);` → `Tail([1, 2])` 就是一个 `Tail<i32>`）；否则由构造器处的显式类型实参确定（`Converter<i32>(\"64\")` —— `class Converter<T>(str);` 无法推断，绑定名不在 `str` 里）。只要有任何一个绑定名未被确定，调用就必须拼写出全部绑定名——全有或全无：`class Pair<A, B>(i32);` 接受 `Pair<i32, str>(7)`，绝不接受 `Pair<i32>(7)`。",

    "**The third-party adapter** (the orphan case, dissolved): `class TheirJson(TheirType);` plus an inherent `encode` hands THEIR type the capability — consumers manufacture `TheirJson(x)` from their own modules; the newtype flag rides the class's surface row like the class itself does.":
        "**第三方适配器**（孤儿情形，就此消解）：`class TheirJson(TheirType);` 加上固有的 `encode`，就把能力交到了他们的类型手里——使用方在自己的模块里制造 `TheirJson(x)`；newtype 标志与类本身一样，搭载在类的接口面行上。",

    "A braced class — even one with a single `inner` field — has no call construction. The positional spelling is what provides the constructor; everything else stays sealed.":
        "花括号类——哪怕只有一个 `inner` 字段——也没有调用构造。位置式写法才提供构造器；其余一切保持封闭。",

    "At runtime the construction mints a real cell: the wrapper is a value like every class.":
        "在运行时，这次构造铸造一个真实的单元：包装器和任何类一样，是一个值。",

    "**Construction is a method call, never a type-call.** User classes construct through their own class methods: `Rect.new(3, 4)`, `Rect.from(other)`, `Version.parse(s)` — see [Classes and constructors](classes.md). The one exception is the newtype decl's own constructor: `class JsonI64(i64);` constructs as the call `JsonI64(64)` (see [Newtypes](classes.md#newtypes-the-one-field-wrapper)). Only builtin surfaces keep other call forms: `bytes.zeroed(n)`, `opaque(v)`, the repeat `[v; n]`, and `Vec<T>.from(..)` (see [Builtin generic types](builtin-generic-types.md)).":
        "**构造是方法调用，绝不是类型调用。** 用户类通过自己的类方法进行构造：`Rect.new(3, 4)`、`Rect.from(other)`、`Version.parse(s)` —— 参见[类与构造器](classes.md)。唯一的例外是新类型（newtype）声明自己的构造器：`class JsonI64(i64);` 以调用 `JsonI64(64)` 的形式构造（参见[新类型](classes.md#newtypes-the-one-field-wrapper)）。只有内置接口面保留其余调用形式：`bytes.zeroed(n)`、`opaque(v)`、重复形式 `[v; n]`，以及 `Vec<T>.from(..)`（参见[内置泛型类型](builtin-generic-types.md)）。",
}  # the newtype sweep's strings; the select/completer entries below are
# carried byte-for-byte by the fold (unchanged msgids).



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
