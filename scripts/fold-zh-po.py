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
    'There is no character type. A `str` iterates as one-codepoint `str`s, and codepoints read as `u32` (`s.code()`, `s.code_at(i)`) — see [string slicing and views](../reference/string-views.md).':
        '没有字符类型。`str` 以单码点 `str` 的形式迭代，码点以 `u32` 读取（`s.code()`、`s.code_at(i)`）——参见[字符串切片与视图](../reference/string-views.md)。',

    'rut has the classic structured statements — `if`/`else`, `while`, `for` — and exactly one match construct: `when`, an exhaustive pattern _expression_. The full rules live in [the reference on control flow](../reference/control-flow.md).':
        'rut 有经典的结构化语句——`if`/`else`、`while`、`for`——以及恰好一个匹配构造：`when`，一个穷尽式的模式_表达式_。完整的规则见[控制流参考](../reference/control-flow.md)。',

    "A class adds two things to a struct: module-private fields and construction gated through class methods. There is no outside literal — the only way to build a class value from outside is to call a class method that chooses to. (Cleanup hooks are not a class privilege: implement `Disposal` for either shape, and the engine calls `dispose` when the value's cell refcount reaches zero — see [Rc, dispose, and identity](../reference/rc-dispose-identity.md).)":
        '类在结构体之上加两样东西：模块私有的字段，以及经由类方法把守的构造。不存在外部字面量——从外部构建类值的唯一途径，是调用一个愿意为你构建的类方法。（清理钩子并不是类的特权：为任一形态实现 `Disposal`，引擎就会在该值的单元引用计数归零时调用 `dispose`——参见[Rc、dispose 与同一性](../reference/rc-dispose-identity.md)。）',

    '**Instance methods spell `self` explicitly** as the first parameter; `mut self` marks methods that write. A computed property is a method (`c.count()`).':
        '**实例方法把 `self` 显式写成第一个参数**；`mut self` 标记会写入的方法。计算属性就是一个方法（`c.count()`）。',

    'rut has no exceptions you catch. Two mechanisms cover everything:':
        'rut 没有你能捕获的异常。两个机制覆盖一切：',

    '`s.slice` deserves a second look: no octets move; the view records a window over the parent, prints, compares by content, iterates, and can re-slice. Codepoint access is spelled with integers — `str.from_code(n)` builds the 1-codepoint `str` for a `u32`. Tokenizing rides `s.scan(from, set)` over a caller-owned `[u8]` class table. See [string slicing and views](../reference/string-views.md).':
        '`s.slice` 值得再看一眼：没有任何八位组移动；视图在父串之上记录一个窗口，它可以打印、按内容比较、迭代，还可以再切片。码点访问以整数书写——`str.from_code(n)` 用一个 `u32` 构建出单码点的 `str`。分词依靠 `s.scan(from, set)`，配一张调用方自备的 `[u8]` 类别表。参见[字符串切片与视图](../reference/string-views.md)。',

    'All output goes through a logger:':
        '所有输出都经过一个 logger：',

    "**Fully static, reified types.** There is no dynamic typing and no gradual typing. Every value's exact type is known to the compiler and carried at runtime — type tests, checked erasure, and host-boundary checks all read the same runtime truth. See [reified types and layouts](reified-types.md).":
        '**完全静态、具体化的类型。**没有动态类型，也没有渐进类型。每个值的确切类型为编译器所知，并在运行时携带——类型测试、受检擦除与宿主边界检查读取的都是同一份运行时事实。参见[具体化类型与布局](reified-types.md)。',

    'There is no exception type. Failures are data in the second element of a record — `(value, err)` — with one documented convention:':
        '没有异常类型。失败是对偶——`(value, err)`——第二个元素里的数据，配一条成文的约定：',

    'There is no runtime layout introspection — the descriptors serve the VM, the checks, and tooling, not userland metaprogramming. Reflection over data (walking fields to serialize) is a library facility built on the same tables; see the reference on [reflection](../reference/reflection.md).':
        '这里没有运行时布局内省——描述符服务于 VM、各项检查与工具链，而不是用户态元编程。对数据做反射（遍历字段以序列化）是一个建立在同一批表之上的库设施；参见关于[反射](../reference/reflection.md)的参考章节。',

    'Everything else — user structs and classes, `Vec`s, trait-typed values, closures — stays inside the VM. A declaration that violates the set is a compile error at the declaration, not a failed call at 2 a.m. A polymorphic crossing seals its value in an erasure box (`opaque(v)` at the call, `opaque.downcast<T>` after), checked, never silent — see [reified types](reified-types.md).':
        '其余一切——用户的 struct 与 class、`Vec`、trait 类型的值、闭包——都留在 VM 内部。违反该集合的声明在声明处就是编译错误，而不是凌晨两点的失败调用。多态跨越把它的值封进擦除盒（调用处 `opaque(v)`，之后 `opaque.downcast<T>`），受检、绝不无声——参见[具体化类型](reified-types.md)。',

    'not special — the conventional construction-method name (`Rect.new(..)`)':
        '并非特殊关键字 —— 只是约定俗成的构造方法名（`Rect.new(..)`）',

    '**Types are PascalCase** — user types and parameterized builtins: `Vec<T>`, `[T]`, `Weak<T>`, `opaque`, `LaunchedFutureHandle<T>`, `Point`, `Drawable`. Scalars and simple buffers stay lowercase: `i32`, `u8`, `f32`, `bool`, `str`, `bytes`.':
        '**类型采用 PascalCase** —— 用户类型与参数化的内置类型：`Vec<T>`、`[T]`、`Weak<T>`、`opaque`、`LaunchedFutureHandle<T>`、`Point`、`Drawable`。标量与简单缓冲区保持小写：`i32`、`u8`、`f32`、`bool`、`str`、`bytes`。',

    'Unannotated = `pub(self)`: nothing leaks unless it says `pub`.':
        '未加注解即等同 `pub(self)`：除非显式写明 `pub`，否则任何东西都不会泄漏。',

    'Absence is `nil` on a nullable `?T`.':
        '缺失用可空类型 `?T` 上的 `nil` 表示。',

    '`.len()` is the sequence member shared by every sequence: `[T]`, `Vec<T>`, `str` (codepoints), `bytes` (octets).':
        '`.len()` 是所有序列共享的序列成员：`[T]`、`Vec<T>`、`str`（码点数）、`bytes`（字节数）。',

    'Construction is the **repeat expression** `[v; n]` — a value and a count. A scalar/`nil` fill is the memset-class op; a ref fill retains the cell handle `n` times — every slot aliases the one cell (the sharing law: the repeat never copies).':
        '构造使用**重复表达式** `[v; n]` —— 一个值和一个数量。标量/`nil` 填充属于 memset 类操作；引用填充会把单元句柄保留 `n` 次 —— 每个槽都是同一单元的别名（共享法则：重复构造绝不复制）。',

    'Explicit type arguments may be spelled at the call: `Vec<i32>.from([1, 2, 3])`. Construction is always a method call (see [Classes and constructors](classes.md)).':
        '显式类型实参可以写在调用处：`Vec<i32>.from([1, 2, 3])`。构造总是方法调用（参见[类与构造器](classes.md)）。',

    '**Classes construct through their own class methods — nothing else is constructible.** No outside literal exists:':
        '**类通过自己的类方法构造 —— 其他任何东西都不可构造。** 不存在外部字面量：',

    '**The receiver is explicit.** An instance method spells its receiver as the first parameter — `fn add(self, x: i32, y: i32)` — and the body reads fields through `self`. A method that mutates declares `mut self` and requires a `let mut` receiver (see [Modules and visibility](modules-and-visibility.md)).':
        '**接收者是显式的。** 实例方法把接收者写为第一个参数 —— `fn add(self, x: i32, y: i32)` —— 体内透过 `self` 读取字段。会做修改的方法声明 `mut self`，并要求 `let mut` 接收者（参见[模块与可见性](modules-and-visibility.md)）。',

    'A method **without** a `self` parameter is a **class method** — invoked on the class itself (`Rect.new(..)`, `Self.new(..)` inside the body). Presence or absence of `self` is the whole distinction. Class methods are ordinary functions: they validate, default, cache, register, or hand out singletons.':
        '**没有** `self` 参数的方法是**类方法** —— 在类本身上调用（`Rect.new(..)`，体内为 `Self.new(..)`）。有或没有 `self` 就是全部区别。类方法就是普通函数：它们做校验、给默认值、缓存、注册或发放单例。',

    '**A computed property is a method** (`c.count()`), and a settable one takes an argument (`c.set_count(n)`). One member kind, one call convention.':
        '**计算属性就是一个方法**（`c.count()`），可写的属性则接受一个参数（`c.set_count(n)`）。一种成员类别，一种调用约定。',

    '**Methods only, no bodies.** No fields, no properties, and **no default implementations, ever** — one member kind, one dispatch candidate per call. Anything that reads like a property is a method.':
        '**只有方法，没有方法体。** 没有字段，没有属性，并且**永远没有默认实现** —— 一种成员类别，每次调用一个分派候选。任何读起来像属性的东西都是方法。',

    '**Widening is nominal and implicit**: a value of `T` widens to `I` exactly where the registry holds a visible `impl I for T` — on assignment, argument passing, and returns. The explicit, greppable form is the trait annotation at the receiving position (`let d: Drawable = s;`). A trait-typed value **cannot be downcast**: use it through the trait, or erase explicitly through `opaque`.':
        '**宽化是名义且隐式的**：`T` 的值在注册表中存在可见的 `impl I for T` 的所有位置宽化为 `I` —— 赋值、传参和返回皆是。显式、可 grep 的形式是接收位置的类型注解（`let d: Drawable = s;`）。trait 类型的值**不能向下转型**：要么透过 trait 使用它，要么通过 `opaque` 显式擦除。',

    'Value size and alignment are implementation details, not a language surface.':
        '值的大小与对齐是实现细节，不是语言接口面。',

    "`b.clone() -> bytes` mints a fresh buffer with `b`'s octets — a one-shot deep copy, the **only copy syntax in the language**. `bytes.from(a)` also deep-copies. Every other type shares on binding, and a divergent value of any other type is unreachable — build a new one instead.":
        '`b.clone() -> bytes` 铸造一个带 `b` 字节的新缓冲 —— 一次性深复制，是**这门语言唯一的复制语法**。`bytes.from(a)` 同样深复制。其他每个类型在绑定时都共享，任何其他类型的分歧值都不可达 —— 需要不同值就新建一个。',

    '`Trap::OutOfFuel` parks the frame exactly like any resumable stop: nothing is unwound. Resumption is `vm.add_fuel(n)` then `vm.resume()` — the frame _is_ the loop state. Fuel is chosen at construction: `add_fuel` is a no-op on an unbounded machine, and the runtime budget is read-only — by design.':
        '`Trap::OutOfFuel` 像任何可恢复停止一样停放帧：没有任何东西被展开。恢复是 `vm.add_fuel(n)` 然后 `vm.resume()` —— 帧 _就是_ 循环状态。燃料在构造时选定：`add_fuel` 在无界机器上是空操作，且运行时预算只读——有意为之。',

    "The crossings cross as **`opaque`**: `opaque(f)` seals a frame on the way out, `opaque.downcast<Future<nil>>(b) -> ?Future<nil>` recovers it on the way in ([opaque — erasure and downcast](opaque.md)). `sleep(ms)` is literally that downcast over the engine's minted sleep frame.":
        '跨越值以 **`opaque`** 的形态跨越：`opaque(f)` 在出去的路上封存帧，`opaque.downcast<Future<nil>>(b) -> ?Future<nil>` 在回来的路上取回它（[opaque —— 擦除与向下转型](opaque.md)）。`sleep(ms)` 正是这个向下转型，作用在引擎铸造的 sleep 帧之上。',

    "A launched future's receipt — `LaunchedFutureHandle<T>` — is its own type, and it is the entire task-management surface. Everything in this chapter is spelled in that vocabulary ([async and await](async.md)).":
        '已启动 future 的回执——`LaunchedFutureHandle<T>`——是它自己的类型，也是任务管理表面的全部。本章的一切都用这套词汇书写（[异步与 await](async.md)）。',

    'The receipt is the join surface: `await h` joins the launched future and produces its completion value. Until the join tier lands, `await h` diagnoses with the join law and the receipt stays non-awaitable.':
        '回执就是 join 表面：`await h` join 已启动的 future 并产出其完成值。在 join 层落地之前，`await h` 会以 join 法则被诊断，回执保持不可 await。',

    'rut→rut names resolve through use paths ([modules and visibility](modules-and-visibility.md)); host→rut entry points are `entry fn` ([loading and the embed loop](loading.md)).':
        'rut→rut 的名字经由 use 路径解析（[模块与可见性](modules-and-visibility.md)）；host→rut 的入口点是 `entry fn`（[加载与嵌入循环](loading.md)）。',

    'A `Template` is `{ parts: [str], args: [opaque] }` — the literal chunks, and the interpolated values **boxed with their runtime types** through the erasure box ([opaque — erasure and downcast](opaque.md)). Construction is an internal native call — the only way to mint one.':
        '一个 `Template` 就是 `{ parts: [str], args: [opaque] }`——字面量块，以及**连同运行时类型一起装盒**、穿过擦除盒的插值（[opaque —— 擦除与向下转型](opaque.md)）。构建是一次内部原生调用——也是铸造它的唯一方式。',

    "There is no global output builtin. All logging goes through a used logger; the host owns the sink, and an uninstalled sink is a silent no-op — a script cannot accidentally spam an embedded host's stdout.":
        '没有全局输出内建。所有日志都经过一个被使用的 logger；汇点由宿主拥有，未安装的汇点是无声的空操作——脚本不可能意外刷屏嵌入式宿主的 stdout。',
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
