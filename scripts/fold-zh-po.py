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
# DESIGN. This sweep (the main convention retires) changes FOUR strings
# non-mechanically — the sentences that described the retired
# convention (the conventional `main` the CLI called); each is rewritten
# against the entry-designation law. Every other changed string is the
# `pub fn main` → `entry fn main` code-span cutover, carried by MECH.
FRESH = {
    # tutorial/modules.md — the loading paragraph: the convention
    # sentence becomes the plain entry shape
    "Module scope contains **declarations only**: `use`, `let`, `fn`, `struct`, `class`, `enum`, `trait`, `impl`. Every statement lives inside a function — and **loading a module executes nothing**. There is no load-time side-effect ordering to reason about; the host loads your module and calls one of its `entry fn`s (`entry fn main` for a plain program).":
        "模块作用域只包含**声明**：`use`、`let`、`fn`、`struct`、`class`、`enum`、`trait`、`impl`。每条语句都住在函数体内——而且**加载模块不执行任何东西**。没有需要推理的加载期副作用顺序；宿主加载你的模块并调用它的某个 `entry fn`（普通程序就是 `entry fn main`）。",
    # tutorial/modules.md — the host-facing-surface close: the
    # convention sentence becomes the designation rule
    "An embedded application drives these entries; `rut run` executes the program's entry designation — exactly one `entry fn` runs it, several take `--entry <name>`. See [the host boundary](../core-concepts/host-boundary.md).":
        "嵌入式应用驱动这些入口；`rut run` 执行程序的入口指派——恰好一个 `entry fn` 时运行它，多个时用 `--entry <name>` 指定。参见[宿主边界](../core-concepts/host-boundary.md)。",
    # reference/modules-and-visibility.md — the entry-points paragraph
    "The embedder loads a module and then explicitly calls an entry function — `entry fn main` for a script, sync or async. `entry fn` publishes a function to the _embedder_; `entry` is orthogonal to visibility and does not combine with `pub`. Consequences of declarations-only loading: no use side-effect ordering, no load-order bugs, deterministic and cheap loads.":
        "嵌入方加载模块后会显式调用一个入口函数 —— 脚本就是 `entry fn main`，同步或异步均可。`entry fn` 把函数发布给 _嵌入方_；`entry` 与可见性正交，且不与 `pub` 组合。只含声明的加载带来的结果：不存在 use 副作用的顺序问题，不存在加载顺序 bug，加载行为确定且开销低。",
    # reference/loading.md — what the host can call: entries, full stop
    "Callable names are exactly the program's **`entry fn`s**. Their signatures were checked against the host-crossing rule at compile time — primitives, `str`, `bytes`, `opaque`, `?T` over a crossing type, and crossing tuples — so a bad surface can never surprise the embedder at call time. An `entry fn -> (?T, err)` decodes positionally at `vm.call` as a `(value, err)` pair (below).":
        "可调用的名字恰好就是程序的 **`entry fn`**。它们的签名在编译期就对照宿主跨越规则检查过——基本类型、`str`、`bytes`、`opaque`、跨越类型之上的 `?T`，以及跨越元组——因此坏表面绝不可能在调用时给嵌入方惊喜。`entry fn -> (?T, err)` 在 `vm.call` 处按位置解码为一个 `(value, err)` 对偶（见下）。",
    # reference/cli.md — the run pipeline row: the designation replaces
    # the retired "run the root's main"
    "load the whole graph, compile it, run the entry designation (below) ([project structure](project-structure.md))":
        "加载整个模块图，编译，运行入口指派（见下）（[项目结构](project-structure.md)）。",
    # reference/cli.md — the --entry flag row
    "name the `entry fn` to run. Without the flag: exactly one `entry fn` runs the program; several refuse to guess (exit 1, naming the set); none reports `no `entry fn` — nothing to run` — a library shape compiles clean, the designation is where the run stops. A named fn that is not an entry is a loud error":
        "指定要运行的 `entry fn`。不带该标志时：恰好一个 `entry fn` 就运行它；多个时拒绝猜测（退出码 1，并列出这一组）；一个都没有则报告 `no `entry fn` — nothing to run`——库形态照常编译，入口指派才是运行停下的地方。指定的名字不是入口时同样报错。",
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
