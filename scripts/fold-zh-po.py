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

# Fresh translations for the strings this sweep changed (the
# `[constructor]` designation docs: the closed marker set grows its
# third member, classes.md grows the designated-construction-surface
# section). Terminology per docs/po/GLOSSARY.md ("designated surface"
# coins 指定表面 there); code spans AND compiler diagnostics
# byte-identical; the extracted markup style is mirrored (`_em_`
# emphasis, `**bold**`; caps emphasis → bold, the catalog's habit).
# Everything
# not in FRESH either keeps its msgstr byte-for-byte (unchanged
# msgid), rides MECH (the mechanical cutover shapes), or folds to ""
# — the English fallback BY DESIGN.
# FRESH is pruned after every landed sweep: once a msgstr is folded
# into zh_CN.po it is kept byte-for-byte by the fold itself, so the
# dict only ever carries translations for the CURRENT sweep's new and
# changed strings. The Phase C sweep's 69 entries landed and were
# pruned; see the git history of this file for them.
FRESH = {
    "**marker = designation** — the bracket markers (`[disposal]`, `[iterable]`, `[constructor]`) are designated surfaces, not polymorphism (`[disposal]` and `[iterable]` are engine hooks; `[constructor]` binds the user's `Type(..)` call form).":
        '**标记 = 指定**——括号标记（`[disposal]`、`[iterable]`、`[constructor]`）是指定表面，不是多态（`[disposal]` 与 `[iterable]` 是引擎钩子；`[constructor]` 绑定的是用户的 `Type(..)` 调用形式）。',
    'The engine\'s own contracts stand alone — no polymorphism machinery, no member-set checks: the bracket markers `[disposal]`, `[iterable]`, and `[constructor]` designate inherent impl members (one per contract per class, the signature checked against the contract — `[disposal]` and `[iterable]` dispatched through a designated slot, never an itable lookup; `[constructor]` bound at the call form `Type(..)`, byte-identical to the member call), and the async protocol lives on two CLOSED `builtin class` rows, `Future<T>` and `RunContext`. The closure IS the wall: futures and cx records are engine-minted only, so a user type cannot BE one — the member-set law cannot see engine identity, and nominal closure is the only spelling of "engine-minted only". See [the async model](async-model.md), [interfaces](../reference/interfaces.md), and [the Rc heap](../reference/rc-heap.md).':
        '引擎自己的契约独立存在——没有多态机器，没有成员集检查：括号标记 `[disposal]`、`[iterable]` 与 `[constructor]` 指定固有 impl 成员（每个类每个契约一个，签名对照契约检查——`[disposal]` 与 `[iterable]` 经指定槽位分派，绝不走 itable 查找；`[constructor]` 绑定在调用形式 `Type(..)` 上，与成员调用逐字节相同），而 async 协议住在两个**封闭的** `builtin class` 行上，`Future<T>` 与 `RunContext`。封闭就是墙：future 与 cx 记录只由引擎铸造，用户类型不可能成为其中之一——成员集法则看不见引擎身份，名义封闭是“仅引擎铸造”的唯一写法。见[异步模型](async-model.md)、[接口](../reference/interfaces.md)与 [the Rc heap](../reference/rc-heap.md)。',
    '`disposal` / `iterable` / `constructor`':
        '`disposal` / `iterable` / `constructor`',
    'the bracket markers — `[disposal] fn` / `[iterable] fn` / `[constructor] fn` designate an inherent impl member as a designated surface (before visibility: `[disposal] pub fn ..`); the set is closed and engine-owned ([interfaces](interfaces.md))':
        '括号标记——`[disposal] fn` / `[iterable] fn` / `[constructor] fn` 将固有 impl 的一个成员指定为指定表面（写在可见性之前：`[disposal] pub fn ..`）；该集合是封闭且引擎所有的（[接口](interfaces.md)）',
    "**Construction is a method call; the call form binds by designation.** User classes construct through their own class methods: `Rect.new(3, 4)`, `Rect.from(other)`, `Version.parse(s)` — see [Classes and constructors](classes.md). A class that designates a `[constructor]` member additionally takes the call form: `Point(1.0, 2.0)` is sugar for the designated member call, byte-identical lowering. The newtype decl's own constructor needs no marker: `class JsonI64(i64);` constructs as the call `JsonI64(64)` — the compiler-provided positional mint (see [Newtypes](classes.md#newtypes-the-one-field-wrapper)). Only builtin surfaces keep the other call forms: `bytes.zeroed(n)`, `opaque(v)`, the repeat `[v; n]`, and `Vec<T>.from(..)` (see [Builtin generic types](builtin-generic-types.md)).":
        '**构造是方法调用；调用形式以指定来绑定。** 用户类通过自己的类方法进行构造：`Rect.new(3, 4)`、`Rect.from(other)`、`Version.parse(s)` —— 参见[类与构造器](classes.md)。指定了 `[constructor]` 成员的类还接受调用形式：`Point(1.0, 2.0)` 是指定成员调用的构造糖——lowering 逐字节相同。新类型（newtype）声明自己的构造器无需标记：`class JsonI64(i64);` 以调用 `JsonI64(64)` 的形式构造——编译器提供的位置铸造（参见[新类型](classes.md#newtypes-the-one-field-wrapper)）。只有内置表面保留其余调用形式：`bytes.zeroed(n)`、`opaque(v)`、重复形式 `[v; n]`，以及 `Vec<T>.from(..)`（参见[内置泛型类型](builtin-generic-types.md)）。',
    "The engine's builtin names — the primitives, `opaque`, `panic`, `type_id<T>()`, `str(x)` — are **ambient**: no `use` is needed for them. The gated names resolve only through `use core::{ .. }`: the const `NAN`, the disposal context `DisposalContext`, the weak reference `Weak<T>`, and the closed async pair (`Future<T>`, `RunContext`) — the engine's weave never needs the import, only source that spells one of the names does. The bracket markers (`[disposal]`/`[iterable]`/`[constructor]`) need no import — the marker word is the designation, not a name ([Host fns and declaration files](host-fns.md)). Package code (`pouch`, `ink`, `nmapset`, ...) mounts only through `use`.":
        '引擎的内建名字——各基本类型、`opaque`、`panic`、`type_id<T>()`、`str(x)`——是**环境性的**：使用它们无需 `use`。被把守的名字只能经 `use core::{ .. }` 解析：常量 `NAN`、disposal 上下文 `DisposalContext`、弱引用 `Weak<T>`，以及封闭的 async 对（`Future<T>`、`RunContext`）——引擎的编织从不需要导入，需要导入的只是拼写了其中某个名字的源代码。括号标记（`[disposal]`/`[iterable]`/`[constructor]`）无需导入——标记词就是指定，不是名字（[宿主 fn 与声明文件](host-fns.md)）。包代码（`pouch`、`ink`、`nmapset`、...）只经 `use` 挂载。',
    '`[constructor]` — the designated construction surface':
        '`[constructor]`——指定的构造表面',
    "A bracket marker designates THE member a surface binds to — the member's name is free; the designation routes the call. Three designated surfaces, one law:":
        '括号标记指定的是表面所绑定的**那一个**成员——成员的名字是自由的；指定路由调用。三个指定表面，一条法则：',
    'marker':
        '标记',
    'caller':
        '调用者',
    'signature owner':
        '签名归属',
    'body compiled':
        '体如何编译',
    'cell death':
        '单元死亡',
    'the engine, at refcount zero':
        '引擎，在引用计数归零时',
    'the engine — `(mut self, cx: DisposalContext)`':
        '引擎——`(mut self, cx: DisposalContext)`',
    'eagerly queued at death ([the Rc heap](rc-heap.md))':
        '死亡时即刻入队（[the Rc heap](rc-heap.md)）',
    '`for (x of it)`':
        '`for (x of it)`',
    'the for-of desugar':
        'for-of 脱糖',
    'the member — its `emit` parameter fixes `E`':
        '成员——它的 `emit` 参数定下 `E`',
    'fused into the loop ([interfaces](interfaces.md))':
        '融合进循环（[接口](interfaces.md)）',
    '`[constructor]`':
        '`[constructor]`',
    'the call form `Type(..)`':
        '调用形式 `Type(..)`',
    'user code':
        '用户代码',
    "the member — its parameters fix the call's shape":
        '成员——它的参数定下调用的形状',
    'byte-identical to `Point.from_xy(..)`':
        '与 `Point.from_xy(..)` 逐字节相同',
    "`[constructor]` designates the class's construction surface — the call form `Type(..)` binds to the one marked member:":
        '`[constructor]` 指定类的构造表面——调用形式 `Type(..)` 绑定到那**一个**被标记的成员：',
    '**Sugar, not a new mechanism.** `Point(1.0, 2.0)` lowers byte-identically to `Point.from_xy(1.0, 2.0)` — the marker changes what the call form binds to, never what the call compiles to. `Point` as a bare (non-call) expression stays an error: the construction surface is the call form `Type(..)`, never the bare name.':
        '**是糖，不是新机制。** `Point(1.0, 2.0)` 的 lower 与 `Point.from_xy(1.0, 2.0)` 逐字节相同——标记改变的是调用形式绑定到谁，绝不是调用被编译成什么。`Point` 作为裸的（非调用）表达式仍是错误：构造表面是调用形式 `Type(..)`，绝不是裸名字。',
    '**One designated constructor per class, on the inherent impl only.** Class bodies stay fields-only; a second `[constructor]` member is refused — "`Point` already carries a `[constructor]` member — `origin` and `from_xy` both designate the construction surface; at most one per class (`Point(..)` must lower to one member)". The member takes no receiver — "a `[constructor]` member takes no receiver — `fn <free>(..) -> Self` (the call `Type(..)` spells the class, not a value)" — and returns `Self` (or `?Self`): "a `[constructor]` member returns `Self` (or `?Self` — the try-construction: `Type(..)` then yields `?Type`)". An unknown bracket word was always refused, and the closed set now names three: "`[fragile]` is not a designated surface — the closed marker set is `[disposal]`, `[iterable]`, and `[constructor]`".':
        '**每个类至多一个被指定的构造器，且只在固有 impl 上。** 类主体保持只有字段；第二个 `[constructor]` 成员会被拒绝——"`Point` already carries a `[constructor]` member — `origin` and `from_xy` both designate the construction surface; at most one per class (`Point(..)` must lower to one member)"。该成员不带接收者——"a `[constructor]` member takes no receiver — `fn <free>(..) -> Self` (the call `Type(..)` spells the class, not a value)"——并且返回 `Self`（或 `?Self`）："a `[constructor]` member returns `Self` (or `?Self` — the try-construction: `Type(..)` then yields `?Type`)"。未知的括号词从来都会被拒绝，而封闭集合现在点名三个："`[fragile]` is not a designated surface — the closed marker set is `[disposal]`, `[iterable]`, and `[constructor]`"。',
    '**`?Self` is try-construction.** A constructor spelled `fn parse(s: str) -> ?Self` makes `Type(..)` answer `?Type` — the call yields the nullable, `nil` on a refused construction, exactly like the method call it is.':
        '**`?Self` 是 try 构造。** 拼写为 `fn parse(s: str) -> ?Self` 的构造器让 `Type(..)` 应答 `?Type`——调用产出可空，构造被拒时得到 `nil`，与它本来就是的那次方法调用完全一致。',
    "**Newtypes never carry it** — `JsonI64(64)` is already the newtype's construction surface, the compiler-provided positional mint; there is nothing for the marker to designate. Structs and enums are refused too — structs construct by literal, and an enum's members are its own immortal singletons.":
        '**新类型（newtype）从不携带它**——`JsonI64(64)` 已经是新类型的构造表面，即编译器提供的位置铸造；标记无可指定之物。结构体与枚举同样被拒绝——结构体以字面量构造，枚举的成员是它自己的不朽单例。',
    '**The seal holds.** Visibility rides the method\'s `pub` — an unannotated (module-private) constructor answers `Type(..)` only inside its own module. Outside, the used-class hint says so: "`Point` constructs through its class methods (`Point.new(..)`) — mark one `[constructor]` to call the class itself". Arity is the member call\'s — the refusal names the member (`Point.from_xy`).':
        '**密封依然成立。** 可见性跟随方法的 `pub`——未加注解的（模块私有）构造器只在自己的模块内应答 `Type(..)`。在模块之外，用到该类处的提示会说明这一点："`Point` constructs through its class methods (`Point.new(..)`) — mark one `[constructor]` to call the class itself"。参数个数沿用成员调用的——拒绝信息点名成员（`Point.from_xy`）。',
    '**Resolution order for `Name(args)`**: a local fn value, then free fns, then the prelude, then the newtype mint, then the designated constructor, then the error — a free fn named `Point` wins.':
        '**`Name(args)` 的解析顺序**：局部 fn 值，然后自由函数，然后 prelude，然后新类型铸造，然后被指定的构造器，最后才是错误——名为 `Point` 的自由函数胜出。',
    '**Generics ride the method-call path.** `Box(3)` infers exactly like `Box.of(3)` — the sugar IS the member call after resolution.':
        '**泛型走方法调用路径。** `Box(3)` 的推断与 `Box.of(3)` 完全一致——解析之后，糖**就是**那次成员调用。',
    "**Named constructors stay live.** `new`, `from_square`, `parse`, `open`, `default` — every existing class method keeps its place; the marker designates one of them as the call form's target, it does not remove the siblings.":
        '**命名构造器依然可用。** `new`、`from_square`、`parse`、`open`、`default`——每个既有类方法都留在原位；标记只是把其中之一指定为调用形式的目标，绝不移除其余兄弟。',
    "A braced class — even one with a single `inner` field — has no call construction of its own: the positional spelling is what provides the newtype's constructor, and a braced class gains the call form only by designating a `[constructor]` member (above). A newtype itself never carries the marker — its mint already IS `Name(v)`; everything else stays sealed.":
        '花括号类——哪怕只有一个 `inner` 字段——也没有自己的调用构造：位置式写法才是新类型构造器的提供者，而花括号类只有指定一个 `[constructor]` 成员（见上）才能获得调用形式。新类型本身从不携带该标记——它的铸造**就是** `Name(v)`；其余一切保持封闭。',
    "**The bracket markers** — `[disposal]`, `[iterable]`, and `[constructor]` designate an inherent impl member. The parser accepts any contextual word in the brackets; the checker validates the engine's CLOSED set, at most one member per contract per class, inherent-members-only, and each contract's signature. The descriptor ABI is a designated slot per class — the engine's release path and the for-of weave read their slot directly, never an itable lookup; `[constructor]`'s slot is bound at the call site — the call form `Type(..)` resolves to the designated member:":
        '**括号标记**——`[disposal]`、`[iterable]` 与 `[constructor]` 指定固有 impl 的一个成员。解析器接受括号里的任何上下文词；检查器校验引擎的**封闭**集合：每个类每个契约至多一个成员、仅限固有成员、并校验每个契约的签名。描述符 ABI 是每类一个指定槽位——引擎的释放路径与 for-of 编织直接读取自己的槽位，绝不走 itable 查找；`[constructor]` 的槽位在调用点绑定——调用形式 `Type(..)` 解析到被指定的成员：',
    "`[disposal]`'s contract: `fn <free>(mut self, cx: DisposalContext)` — the engine calls it when a value of the type reaches refcount zero ([the Rc heap](rc-heap.md)); the target must be a CONCRETE struct or class (the row keys the cell's type id). `[iterable]`'s contract is the iteration protocol above. `[constructor]`'s contract: `fn <free>(..) -> Self` (or `?Self`) on a class's inherent impl — the one user-invoked surface: `Type(..)` lowers byte-identically to the designated member call, the seal rides the method's `pub`, and newtypes are refused (their positional mint already IS `Name(v)`) — see [Classes and constructors](classes.md).":
        '`[disposal]` 的契约：`fn <free>(mut self, cx: DisposalContext)`——当该类型的值的引用计数归零时，引擎调用它（[the Rc heap](rc-heap.md)）；目标必须是具体的结构体或类（行以单元的类型 id 为键）。`[iterable]` 的契约即上文迭代协议。`[constructor]` 的契约：类固有 impl 上的 `fn <free>(..) -> Self`（或 `?Self`）——唯一由用户调用的表面：`Type(..)` 的 lower 与被指定成员的调用逐字节相同，密封跟随方法的 `pub`，新类型被拒绝（它们的位置铸造**就是** `Name(v)`）——参见[类与构造器](classes.md)。',
    "The builtin fns, primitives, and containers are in scope in every compilation unit; no `use` is needed. A `use core::{ … };` statement stays legal but is redundant for them. The exceptions — the **import-gated** spellings, resolving only through `use core::{ .. }`: the const `NAN`, the disposal context `DisposalContext`, the weak reference `Weak<T>`, and the closed async pair (`Future<T>`, `RunContext`) — the engine's weave itself never needs the import, only source that spells the names (a `cx: T` parameter, a launcher's `f: Future<T>`, a `downcast<Future<..>>`). The bracket markers (`[disposal]`/`[iterable]`/`[constructor]`) need NO import — the marker word is the designation, not a name. The two builtin spellings (ambient vs import-gated) are documented in [Host fns and declaration files](host-fns.md).":
        '内建 fn、基本类型与容器在每个编译单元内都在作用域中；无需 `use`。`use core::{ … };` 语句仍然合法，但对它们是多余的。例外——**导入把守的**拼写，只能经 `use core::{ .. }` 解析：常量 `NAN`、disposal 上下文 `DisposalContext`、弱引用 `Weak<T>`，以及封闭的 async 对（`Future<T>`、`RunContext`）——引擎的编织本身从不需要导入，需要导入的只是拼写了这些名字的源代码（一个 `cx: T` 参数、启动器的 `f: Future<T>`、一次 `downcast<Future<..>>`）。括号标记（`[disposal]`/`[iterable]`/`[constructor]`）无需导入——标记词就是指定，不是名字。两种内建拼写（环境性 vs 导入把守）记载于[宿主 fn 与声明文件](host-fns.md)。',
    "`fn <free>(..) -> Self` (or `?Self`) on a class's inherent impl":
        '类固有 impl 上的 `fn <free>(..) -> Self`（或 `?Self`）',
    'the construction surface: the call form `Type(..)` binds to the ONE designated member — user-invoked, the lowering byte-identical to `Type.<name>(..)`; one per class, no receiver; newtypes are refused — their positional mint already IS `Name(v)` ([classes](classes.md))':
        '构造表面：调用形式 `Type(..)` 绑定到**那一个**被指定的成员——由用户调用，lowering 与 `Type.<name>(..)` 逐字节相同；每类一个，不带接收者；新类型被拒绝——它们的位置铸造**就是** `Name(v)`（[类与构造器](classes.md)）',
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
