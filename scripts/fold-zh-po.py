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
# DESIGN. This sweep (the history framing is scrubbed — the book
# speaks in present tense) rewrites EIGHT prose msgids; the removed
# spellings/tables and the deleted paragraphs fold away as obsolete.
# Each fresh msgstr is the old translation with the history clause
# dropped, wording moved to the present tense.
FRESH = {
    # ---- the history-scrub sweep: present tense, no removed spellings ----
    # Terminology per docs/po/GLOSSARY.md: 可空 (nullable), 枚举 (enum),
    # 单元 (cell), 引用 (reference), 标准库 (standard library).

    # examples/06-github-viewer-cli.md — the `del` verb keeps only the
    # wire-canonicalization clause ("the wire still sees DELETE").
    "The std `http` package is **async-only and unsuffixed**: only the operations that really wait are async points, everything else is sync construction sugar. The five verbs are build sugars (`get`/`post`/ `put`/`patch`/`del` — the DELETE verb spells `del`; the wire still sees canonical `DELETE`), and `send(cx)` is THE async point, resolving **at headers** — the wire body stays unread ([the async model](../core-concepts/async-model.md)):":
        "std `http` 包**只异步且无后缀**：真正要等的操作才是异步点，其余一切都是同步的构建语法糖。五个动词都是构建糖（`get`/`post`/ `put`/`patch`/`del` ——DELETE 动词拼作 `del`；线上看到的仍是规范的 `DELETE`），而 `send(cx)` 是那个 THE 异步点，**在头部就**解析——线上主体保持未读（[异步模型](../core-concepts/async-model.md)）：",

    # examples/06-github-viewer-cli.md — `out` (stdout), the removed
    # core name aside is gone.
    "The HTTP bodies install through `http::pkg()` (the reqwest lane); the example's own rows are CLI I/O only — `out` (stdout), `eprint`, the file pair, and `exit` — declared in the example-local `rgh_host` decl package and verified against the bindings at boot ([Host fns and declaration files](../reference/host-fns.md)). One consequence: `rut run` cannot host rgh itself — its rows are example-local — but ordinary HTTP programs do run under `rut run`, which mounts the std http pair by presence.":
        "HTTP 的函数体通过 `http::pkg()` 安装（reqwest 车道）；示例自己的行只有 CLI I/O——`out`（标准输出）、`eprint`、文件对和 `exit`——在示例本地的 `rgh_host` 声明包里声明，并在启动时对照绑定校验（[宿主函数与声明文件](../reference/host-fns.md)）。一个后果：`rut run` 无法自己承载 rgh——它的行是示例本地的——但普通 HTTP 程序可以在 `rut run` 下运行，它按存在性挂载 std http 对。",

    # reference/primitive-types.md — the no-eager-copy law stated
    # directly, no removed spelling named.
    "There is no eager copy: **`bytes.clone()` is the one copy escape hatch**. There is no `&`/`*` syntax anywhere. `==` on cells is **identity** (the raw slot compare); `str`/`bytes` compare by content — the full table is in [Rc, dispose, and identity](rc-dispose-identity.md).":
        "没有急切复制：**`bytes.clone()` 是唯一的复制逃生口**。任何地方都没有 `&`/`*` 语法。单元上的 `==` 是**同一性**（identity）比较（原始槽位比较）；`str`/`bytes` 按内容比较 —— 完整表格见[Rc、dispose 与同一性](rc-dispose-identity.md)。",

    # reference/builtin-generic-types.md — the absence/errors lead-in
    # keeps only the present-tense law (the stdlib removed-surface
    # pointer is gone with the table it pointed at).
    "There are no `Option`/`Result` builtins — the spellings are ordinary identifiers, and an unresolved use diagnoses as the unknown name it is:":
        "没有 `Option`/`Result` 内置物——这些写法是普通标识符，未解析的使用会被诊断为那个未知名称：",

    # reference/builtin-generic-types.md — the `==` law in present
    # tense, aligned with the page's slot-identity rule.
    "A nullable/enum value is not compared structurally with `==` — test it with `when`, a `nil`/`!= nil` guard, or the payload.":
        "可空值/枚举值不用 `==` 做结构化比较——用 `when`、`nil`/`!= nil` 守卫或其载荷来检验。",

    # reference/rc-dispose-identity.md — sharing stated directly, the
    # removed-spellings clause is gone.
    "There is no eager copy of any composite — bindings share by reference. **`bytes.clone()` is the one copy escape hatch**: a one-shot deep copy of a buffer's octets. Every other type shares on binding; a divergent value of any other type is unreachable — build a new one instead.":
        "任何复合值都没有急切复制——绑定按引用共享。**`bytes.clone()` 是唯一的复制逃生口**：对缓冲区字节的一次性深复制。其他每个类型在绑定时都共享；任何其他类型的分歧值都不可达 —— 需要不同值就新建一个。",

    # reference/weak-refs.md — the construction bullet ends at
    # "refuse"; the retired type-call parenthetical is gone.
    "Construction is a **class method** with admission at the instantiation: `T` must be a **reference type**. `Weak<i32>` and `Weak.new(some_fn)` diagnose; primitives and `fn` values refuse.":
        "构造是**类方法**，在实例化时准入：`T` 必须是**引用类型**。`Weak<i32>` 与 `Weak.new(some_fn)` 会被诊断；基本类型与 `fn` 值拒绝。",

    # reference/host-fns.md — the `any` bullet in present tense.
    "**`any` is not in the language.** An `any` spelling fails at resolution as an unknown type — there is no `any` type to write. Seal polymorphic values with `opaque(v)` / `opaque.downcast<T>(v)` ([opaque](opaque.md)).":
        "**`any` 不在这门语言里。**写出 `any` 会在名称解析阶段以未知类型失败——也不存在可写的 `any` 类型。用 `opaque(v)` / `opaque.downcast<T>(v)` 封存多态值（[opaque](opaque.md)）。",
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
