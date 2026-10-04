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

# Fresh translations for the strings this sweep changed (the std
# `[constructor]` adoption: `Logger` / `StringBuilder` / `HttpClient` /
# `HashMap` / `HashSet` designate `new`, and the doc-spelling sweep
# leads with the call form — `Logger("t")` — while naming the
# equivalence). Terminology per docs/po/GLOSSARY.md ("designated
# surface" coins 指定表面 there; 构造器 / 调用形式 / 命名构造器 per the
# landed phase-2 entries); code spans byte-identical; the extracted
# markup style is mirrored (`**bold**`, the catalog's habit).
# Everything not in FRESH either keeps its msgstr byte-for-byte
# (unchanged msgid), rides MECH (the mechanical cutover shapes), or
# folds to "" — the English fallback BY DESIGN. Code blocks fold to ""
# (the untranslated set is code-only).
# FRESH is pruned after every landed sweep: once a msgstr is folded
# into zh_CN.po it is kept byte-for-byte by the fold itself, so the
# dict only ever carries translations for the CURRENT sweep's new and
# changed strings. The phase-2 sweep's 40 `[constructor]`-designation
# entries landed and were pruned; see the git history of this file for
# them.
FRESH = {
    "**`Logger(\"hello\")`** constructs the logger through the class's designated `[constructor]` member — the same call as `Logger.new(\"hello\")`, spelled as the call form ([classes](../reference/classes.md)).":
        '**`Logger("hello")`** 经由类指定的 `[constructor]` 成员构造 logger——与 `Logger.new("hello")` 是同一次调用，以调用形式拼写（[类与构造器](../reference/classes.md)）。',
    'Both classes designate `new` — `HashMap()` / `HashSet()` spell the construction (the same call as `HashMap.new()` / `HashSet.new()`); `with_capacity` stays a named constructor.':
        '两个类都指定 `new`——`HashMap()` / `HashSet()` 拼写构造（与 `HashMap.new()` / `HashSet.new()` 是同一次调用）；`with_capacity` 仍是命名构造器。',
    "Keys come from a fixed set — integers, `bool`, `str`, `bytes` (no floats: they have no stable equality contract) — hashed by the host; a user-defined key escapes by encoding canonically to `bytes`. A hit returns the stored cell, not a copy. There is no iteration surface: maps and sets answer questions, they don't walk. Construction spells the call form — `HashMap()` / `HashSet()` are the designated `new` (the same call as `HashMap.new()`); `with_capacity(n)` stays named.":
        '键来自一个固定集合——整数、`bool`、`str`、`bytes`（没有浮点数：它们没有稳定的相等契约）——由宿主做哈希；用户自定义的键以规范编码成 `bytes` 的方式破例。命中返回的是存储的单元，不是副本。没有迭代表面：map 和 set 回答问题，不做遍历。构造以调用形式拼写——`HashMap()` / `HashSet()` 是被指定的 `new`（与 `HashMap.new()` 是同一次调用）；`with_capacity(n)` 仍是命名构造器。',
    'The package is **one bare identifier**; the names are one or more idents. rut is fully statically typed: the compiler resolves every used name and knows from usage whether it lands in type position (`Vec` in an annotation) or value position (`Logger(..)`), so there is nothing for the user to annotate. An unreferenced use name is a lint, not an error.':
        '包名是**单个裸标识符**；名称则是一个或多个标识符。rut 是完全静态类型的：编译器会解析每个被使用的名称，并根据用法判断它处于类型位置（注解中的 `Vec`）还是值位置（`Logger(..)`），因此用户无需编写任何注解。未被引用的 use 名称只是一条 lint，不是错误。',
    '`Logger("app")` constructs through the designated `new` — the call form and `Logger.new("app")` are the same call.':
        '`Logger("app")` 经由被指定的 `new` 构造——调用形式与 `Logger.new("app")` 是同一次调用。',
    '`StringBuilder()` is the designated `new` (the same call as `StringBuilder.new()`); `with_cap(n)` pre-sizes and stays named.':
        '`StringBuilder()` 是被指定的 `new`（与 `StringBuilder.new()` 是同一次调用）；`with_cap(n)` 预先定容，仍是命名构造器。',
    '`http_host` declares the transport rows (three async, five sync readbacks); `http` wraps them in `HttpClient` / `RequestBuilder` / `Request` / `Response` / `ByteStream` — async only at the points that really wait, and `HttpClient()` constructs through the designated `new`:':
        '`http_host` 声明传输各行（三个异步，五个同步读回）；`http` 把它们包装成 `HttpClient` / `RequestBuilder` / `Request` / `Response` / `ByteStream`——只在真正等待的点是异步的，而 `HttpClient()` 经由被指定的 `new` 构造：',
    "`new` is not special syntax — just the conventional primary-constructor name (`from`, `parse`, `open`, `default` are its siblings); it is an ordinary identifier. Try-construction returns the nullable: a class method `fn parse(s: str) -> ?Version` answers `nil` on failure. A class may designate its primary constructor with `[constructor]` — then the call form spells the construction too: std's `Logger(\"t\")` is the same call as `Logger.new(\"t\")` (the `[constructor]` section below).":
        '`new` 不是特殊语法 —— 只是约定俗成的主构造方法名（`from`、`parse`、`open`、`default` 是它的同辈）；它是一个普通标识符。尝试式构造返回可空类型：类方法 `fn parse(s: str) -> ?Version` 在失败时应答 `nil`。类可以用 `[constructor]` 指定它的主构造器——此后调用形式也能拼写构造：std 的 `Logger("t")` 与 `Logger.new("t")` 是同一次调用（见下文的 `[constructor]` 一节）。',
    'grow from small — the designated `[constructor]`: `StringBuilder()` spells it':
        '从小处开始增长——被指定的 `[constructor]`：`StringBuilder()` 拼写的就是它',
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
