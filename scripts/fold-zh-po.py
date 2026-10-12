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

# Fresh translations for the strings this sweep changed (the `rut new`
# documentation: cli.md gains the usage row and the `## new` section —
# purpose sentence, behavior table, the two-file text listing;
# first-program.md gains the scaffold shortcut after the manual
# walkthrough; installation.md's usage line gains `new <dir>` and the
# Try-it gains the one-step-equivalent sentence; stdlib.md's swappable
# table gains the `flow` row. The std-v9 pin sweep touches only code
# blocks — those fold to ""). Terminology per docs/po/GLOSSARY.md
# (脚手架 / 拒绝覆盖 / 基名 / 派生名 / 可换 std 集合 / 推送管线 /
# 固定值 / 退出码); code spans byte-identical (`rut new <dir>`,
# `rut.jsonc`, `mod.rut`, `@std-v9`); link text translated, targets
# untouched ([rut CLI](../reference/cli.md), [你的第一个 rut 程序]);
# full-width punctuation in prose. Everything not in FRESH either
# keeps its msgstr byte-for-byte (unchanged msgid), rides MECH (the
# mechanical cutover shapes), or folds to "" — the English fallback BY
# DESIGN. Code blocks fold to "" (the untranslated set is code-only).
# FRESH is pruned after every landed sweep: once a msgstr is folded
# into zh_CN.po it is kept byte-for-byte by the fold itself, so the
# dict only ever carries translations for the CURRENT sweep's new and
# changed strings. This sweep's seventeen entries landed and were
# pruned; see the git history of this file.
FRESH = {
    # phase(9) definite-return: the functions reference page's new
    # bullet (the law + what the analysis sees). Code spans byte-
    # identical (`return`, `break`/`continue`, `if`/`else`, `when`,
    # `while (true)`); the diagnostic text stays English inside the
    # sentence, as quoted; full-width punctuation in prose.
    "A function WITH a result arrow must return on every path: a body that can "
    "fall off the end diagnoses — `missing return: 'f' is '-> str' but a path "
    "reaches the end of the body`, naming the function and its arrow — instead of "
    "handing the caller a never-written value. The analysis sees `return`, "
    "`break`/`continue`, an `if`/`else` whose arms both return, an exhaustive "
    "`when` whose arms all return, and a `while (true)` with no `break` bound to "
    "it (the scan-loop shape); dead statements after a `return` are still just "
    "dead code. The same law covers the closure block form.":
    "带结果箭头的函数必须在每条路径上返回：函数体可能执行到末尾自然结束时"
    "会诊断——`missing return: 'f' is '-> str' but a path reaches the end of "
    "the body`，点名函数及其箭头——而不是把一个从未写入的值交给调用方。"
    "该分析能看到 `return`、`break`/`continue`、两个分支都返回的 "
    "`if`/`else`、所有臂都返回的穷尽 `when`，以及没有绑定到自身的 "
    "`break` 的 `while (true)`（扫描循环形态）；`return` 之后的死语句"
    "仍然只是死代码。闭包块形式同样适用这条法则。",
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
