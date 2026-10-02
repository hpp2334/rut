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

# Fresh translations for the strings this sweep changed non-mechanically
# (terminology per docs/po/GLOSSARY.md; code spans byte-identical;
# markdown structure load-bearing). Everything not in FRESH either
# keeps its msgstr byte-for-byte (unchanged msgid), rides MECH (the
# mechanical cutover shapes), or folds to "" — the English fallback BY
# DESIGN. This sweep (generic foreign traits cross modules — the v1
# gates lift) changes FOUR strings: three are the traits pages'
# placement/satisfaction bullets gaining the "the one cross-module
# restriction" clause, one is the new crossing bullet.
FRESH = {
    # tutorial/traits.md — the placement bullet: the only-rule clause
    # and the generic crossing sentence extend the old translation.
    "**Placement:** an impl may live in a module of the trait's package or the type's package — at least one side must be yours. You cannot implement two foreign types to each other. This is the **only** cross-module rule: a foreign trait crosses freely for your own type, generic traits included — `impl<T> Wrap<T> for Box2<T>` against a used pkg's `trait Wrap<T>` is legal, and so is spelling `Wrap<i32>` as a parameter type.":
        "**位置：**impl 可以放在 trait 所在包或类型所在包的模块里——至少有一侧必须属于你。你不能给两个外部类型互相实现。这是**唯一的**跨模块规则：外部 trait 可以自由地为你自己的类型实现，泛型 trait 也不例外——对着 `use` 进来的包写 `impl<T> Wrap<T> for Box2<T>`（其 `trait Wrap<T>`）是合法的，把 `Wrap<i32>` 拼写成参数类型同样合法。",
    # core-concepts/traits-and-dispatch.md — the placement bullet: the
    # closing clause names placement the only cross-module restriction.
    "**Placement is pair-local.** A trait impl may live in the trait's package or the type's package — at least one side of every `(trait, type)` pair must be yours. Implementing two foreign types' pairing is rejected outright; there is no orphan rule beyond that — placement is the only cross-module restriction, and a foreign trait crosses freely for a local type (generic traits included).":
        "**放置位置对组合局部。**trait impl 可以放在 trait 所在的包或类型所在的包——每个 `(trait, type)` 组合至少有一侧属于你自己。实现两个外部类型的组合会被直接拒绝；除此之外没有别的孤儿规则——放置是唯一的跨模块限制，外部 trait 可以为本地类型自由实现（泛型 trait 也不例外）。",
    # reference/traits.md — the satisfaction bullet: the closing clause
    # names the placement rule the only cross-module impl restriction.
    "**Satisfaction is nominal.** A type that declares every member by shape is still not an `I` until some module writes `impl I for T`. There is no duck typing and no orphan rule beyond placement: for every `impl Trait for Type`, **at least one of `Type` or `Trait` must be defined in the current pkg** — both foreign is a compile error. Builtin types (`[T]`, the primitives, `?T`, `opaque`) are in no pkg: only a _local trait_ may be implemented for a builtin. This placement rule is the **only** cross-module impl restriction — a foreign trait crosses freely for a local type, generic or not.":
        "**满足是名义性的。** 一个按形状声明了全部成员的类型，在某些模块写出 `impl I for T` 之前仍不是 `I`。没有鸭子类型，除了位置之外也没有孤儿规则：对每个 `impl Trait for Type`，**`Type` 或 `Trait` 至少一个必须定义在当前包** —— 两者都是外来的就是编译错误。内建类型（`[T]`、各原语、`?T`、`opaque`）不属于任何包：只有_本包的 trait_ 才能为内建类型实现。这条放置规则是**唯一的**跨模块 impl 限制——外部 trait 可以为本地类型自由实现，泛型与否皆可。",
    # reference/traits.md — the new crossing bullet.
    "**Generic traits cross modules.** A consumer implements a foreign generic trait for its own type — `impl<T> Wrap<T> for Box2<T>` against a `use`d pkg's `trait Wrap<T>` — and spells the trait in type position (`fn describe(w: Wrap<i32>) -> i32`). The trait's declaration crosses the used pkg's surface, each type-argument list instantiates it where it is used, and dispatch is the ordinary law: one concrete origin binds statically, merged origins consult the vtable. The orphan rule above is the only gate.":
        "**泛型 trait 跨模块。**使用者为自己的类型实现一个外部的泛型 trait——对着 `use` 进来的包的 `trait Wrap<T>` 写 `impl<T> Wrap<T> for Box2<T>`——并把该 trait 拼写进类型位置（`fn describe(w: Wrap<i32>) -> i32`）。trait 的声明随被使用包的表面跨越而来，每个类型实参列表在使用的位置实例化它，而分派仍是常规法则：单一具体来源静态绑定，来源合并则查 vtable。上面的孤儿规则是唯一的门。",
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
