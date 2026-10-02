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
# DESIGN. This sweep (the crossing rule rejects named records) changes
# ONE string non-mechanically — the host-fns crossing-set bullet that
# claimed `host struct` records cross; it now states the true set and
# points the tuple lane at the entry-fn surface.
FRESH = {
    # reference/host-fns.md — the crossing-set bullet: records never
    # cross, the anonymous-tuple lane is the entry-fn surface's
    "**The crossing set** — `host fn` signatures are concrete over: nil, the primitives, `str`, `bytes`, and `opaque`. Returns may additionally use the answer optionals `?str` / `?bytes` / `?opaque`. Named records never cross — a `host struct` decl is surface, not a signature type — and the anonymous-tuple lane belongs to the `entry fn` surface ([value boundary](value-boundary.md)). Everything else (user classes, `Vec<T>`, `[T]`, trait objects, closures) is a compile error on the declaration.":
        "**跨越集合**——`host fn` 签名在以下类型上是具体的：nil、基本类型、`str`、`bytes` 和 `opaque`。返回值还可以使用应答可空类型 `?str` / `?bytes` / `?opaque`。具名记录永不跨越——`host struct` 声明只是表面，不是签名类型——而匿名元组通道属于 `entry fn` 表面（[值边界](value-boundary.md)）。其余一切（用户类、`Vec<T>`、`[T]`、trait 对象、闭包）都是声明上的编译错误。",
    # reference/host-fns.md — the crossing-set bullet: records never
    # cross, the anonymous-tuple lane is the entry-fn surface's
    "**The crossing set** — `host fn` signatures are concrete over: nil, the primitives, `str`, `bytes`, and `opaque`. Returns may additionally use the answer optionals `?str` / `?bytes` / `?opaque`. Named records never cross — a `host struct` decl is surface, not a signature type — and the anonymous-tuple lane belongs to the `entry fn` surface ([value boundary](value-boundary.md)). Everything else (user classes, `Vec<T>`, `[T]`, trait objects, closures) is a compile error on the declaration.":
        "**跨越集合**——`host fn` 签名在以下类型上是具体的：nil、基本类型、`str`、`bytes` 和 `opaque`。返回值还可以使用应答可空类型 `?str` / `?bytes` / `?opaque`。具名记录永不跨越——`host struct` 声明只是表面，不是签名类型——而匿名元组通道属于 `entry fn` 表面（[值边界](value-boundary.md)）。其余一切（用户类、`Vec<T>`、`[T]`、trait 对象、闭包）都是声明上的编译错误。",
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
