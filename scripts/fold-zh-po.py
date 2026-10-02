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
