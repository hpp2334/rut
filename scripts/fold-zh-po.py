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

# Fresh translations for the strings this sweep changed (plan5-phase(4)
# book: `mut` in loop headers — the mut-binding law now covers both `for`
# forms, so the "loop-owned induction variable" prose is gone and the
# law prose + grammar/toggle blocks land in reference/control-flow.md,
# tutorial/control-flow.md, everything-is-a-value.md,
# by-reference-and-nullable.md, functions-closures-generics.md).
# Terminology per docs/po/GLOSSARY.md (mut 绑定法则 / 只读视图 /
# 写访问权 / 循环头 / 绑定); code spans byte-identical (`for (let mut i
# = 0; ...)`, `let`, `mut`); diagnostic text stays English inside the
# sentence; full-width punctuation in prose. Everything not in FRESH
# either keeps its msgstr byte-for-byte (unchanged msgid) or folds to
# "" — the English fallback BY DESIGN. Code blocks fold to "" (the
# untranslated set is code-only). FRESH is pruned after every landed
# sweep; the phase(11) every-field-covered entries landed and were
# pruned; see the git history of this file.
FRESH = {
    # reference/control-flow.md — the indexed form's lead-in and the law.
    "Indexed form — the C shape:":
    "索引形式 —— C 的形状：",

    "Both headers declare a **normal binding**, and the mut-binding law "
    "gates them like any `let`: assigning the loop variable — in the "
    "body, or in the indexed form's update clause — requires `mut` in "
    "the header:":
    "两种形式的循环头都声明一个**普通绑定**，mut 绑定法则像对待任何 `let` "
    "一样把守它们：给循环变量赋值——无论是在循环体里，还是在索引形式的更新"
    "子句里——都需要循环头里的 `mut`：",

    # tutorial/control-flow.md — the two loop-variable paragraphs.
    "The loop variable is a fresh binding each iteration — and a "
    "**normal** one: assigning it in the body takes `mut` in the header, "
    "like any `let`.":
    "循环变量每次迭代都是崭新的绑定——而且是**普通的**绑定：在循环体里给"
    "它赋值需要在循环头写 `mut`，与任何 `let` 一样。",

    "**Indexed `for`** is the C shape. The header is a normal binding "
    "too: the update clause assigns the counter every iteration, so a "
    "written-to counter is spelled `for (let mut i = 0; ...)` — plain "
    "`let` there is a compile error.":
    "**索引式 `for`** 是 C 的形状。它的循环头同样是普通绑定：更新子句每"
    "一轮都给计数器赋值，所以要被写入的计数器写作 `for (let mut i = 0; "
    "...)`——那里用纯 `let` 是编译错误。",

    # core-concepts/everything-is-a-value.md — the loop-variables bullet.
    "**Loop variables share elements; the binding is one variable.** A "
    "`for (let x of xs)` loop hands you the stored element: plain `let` "
    "is a read-only view, and write access follows the header's `mut` "
    "like any binding — a `let mut` loop variable's writes mutate the "
    "sequence. The loop variable is ONE binding reassigned per "
    "iteration — `for..of` is sugar for a `while`\\-shaped loop, and the "
    "capture law treats it like any other ref-headed binding.":
    "**循环变量共享元素；绑定是同一个变量。** `for (let x of xs)` 循环把"
    "存储的元素交给你：纯 `let` 是只读视图，写访问权跟随循环头的 `mut`，"
    "与任何绑定一样——`let mut` 循环变量的写入会改动序列本身。循环变量是"
    "每一轮被重新赋值的同一个绑定——`for..of` 是 `while`\\-形循环的语法糖，"
    "捕获法则像对待其他以引用为头的绑定一样对待它。",

    # reference/by-reference-and-nullable.md — the flat-slots bullet.
    "Primitive elements in a `[T]` stay flat slots; composite elements "
    "are handle slots. `for (let x of xs)` yields the **shared** element, "
    "not a per-element copy — a cell element writes through to the "
    "sequence, and writes through it are gated by the header's `mut`, "
    "like any binding.":
    "`[T]` 中的基本类型元素保持扁平槽；复合元素是句柄槽。`for (let x of "
    "xs)` 产出**共享的**元素，而不是逐元素复制 —— 单元元素的写入直接穿透"
    "到序列，而穿过它的写入由循环头的 `mut` 把守，与任何绑定一样。",

    # reference/functions-closures-generics.md — the capture-law bullet
    # (old translation's unchanged head carried over; the parenthetical
    # gains the loop variable's write-access clause).
    "**The capture law**: primitives, `nil`, and `fn` values copy — a "
    "captured scalar is the closure's own slot, and reassigning it inside "
    "the closure never moves the original. Ref-headed values (records, "
    "`Vec`, arrays, `str`/`bytes`, `?T`, enums) cross as handles: writes "
    "through the captured handle reach the shared cell, and so does a "
    "whole-value REASSIGNMENT — a captured-and-reassigned binding is "
    "promoted to a hidden shared-slot cell, so both frames stay linked "
    "for the binding's whole scope (a stash-then-reassign, a reassign "
    "inside the closure, and the `for..of` loop variable — one binding "
    "reassigned per iteration, its write access gated by the header's "
    "`mut` — all follow this one law). Refcounting keeps captures alive; "
    "a closure is itself a shared cell value.":
    "**捕获法则**：标量、`nil` 和 `fn` 值按复制走——被捕获的标量是闭包自"
    "己的槽位，在闭包内重新赋值绝不会移动原绑定。以引用为头的值（记录、"
    "`Vec`、数组、`str`/`bytes`、`?T`、枚举）以句柄跨越：穿过被捕获句柄"
    "的写入抵达共享单元，整体重新赋值也是如此——一个既被捕获又被重新赋值"
    "的绑定会晋升为隐藏的共享槽位单元，两个帧在整个绑定作用域内保持联动"
    "（先暂存再重赋值、闭包内重赋值、以及 `for..of` 循环变量——每一轮被重"
    "新赋值的同一个绑定，其写访问权由循环头的 `mut` 把守——都遵循这同一条"
    "法则）。引用计数让捕获保持存活；闭包本身就是一个共享单元值。",
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
