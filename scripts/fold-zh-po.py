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

# Fresh translations for the strings this sweep changed (phase(11)
# docs: the every-field-covered + duplicate-field literal laws and the
# unknown-import law — structs.md, classes.md, types.md,
# values-and-variables.md, modules-and-visibility.md,
# tutorial/modules.md). Terminology per docs/po/GLOSSARY.md
# (字段全覆盖法则 / 重复字段 / 未知导入 / 环境自带 / 声明初始化器);
# code spans byte-identical (`tag: str = "..."`, `{}`, `use`,
# `` `ink` has no `Nope` — unknown import ``, the missing/duplicate
# field diagnostics — diagnostic text stays English inside the
# sentence); link text translated, targets untouched; full-width
# punctuation in prose. Everything not in FRESH either keeps its
# msgstr byte-for-byte (unchanged msgid) or folds to "" — the English
# fallback BY DESIGN. Code blocks fold to "" (the untranslated set is
# code-only). FRESH is pruned after every landed sweep; the phase(9)
# definite-return entry landed and was pruned; see the git history of
# this file.
FRESH = {
    # structs.md — the law bullets.
    "**The every-field-covered law**: a literal must cover **every** field — "
    "spelled in the literal, or covered by the field's declared initializer "
    "(`tag: str = \"...\"`). Omitting a field that has no initializer is a "
    "compile error — ``missing field `b` in the `P` literal (the "
    "every-field-covered law)`` — the silent zero-fill default is gone.":
    "**字段全覆盖法则**：字面量必须覆盖**每一个**字段——要么在字面量里写出，"
    "要么由该字段的声明初始化器（`tag: str = \"...\"`）覆盖。省略一个没有初"
    "始化器的字段是编译错误——``missing field `b` in the `P` literal (the "
    "every-field-covered law)``——悄无声息的零值填充默认已不复存在。",

    "A field spelled twice is a compile error — ``duplicate field `a` in the "
    "`P` literal`` — there is no last-wins.":
    "把一个字段写两遍是编译错误——``duplicate field `a` in the `P` "
    "literal``——不存在“后者胜”（last-wins）。",

    "An omitted field with an initializer takes it; `{}` with all-initialized "
    "fields is legal.":
    "带初始化器的字段被省略时取其初始化器；当所有字段都有初始化器时，`{}` "
    "是合法的。",

    # classes.md — the Self-literal bullet, extended with the two laws.
    "**The `Self { field: expr, .. }` literal is the class-private "
    "construction** — legal anywhere inside the class body (class methods and "
    "instance methods alike); the class name spells it inside the body too "
    "(`Rect { .. }`). The literal must initialize every field without an "
    "initializer — the every-field-covered law, structs and classes alike — "
    "and a field spelled twice is an error; field initializers run for "
    "omitted fields. Private fields are settable in the literal — inside the "
    "class body only. That privacy _is_ the seal: outside code can build a "
    "class value only by calling a class method that chooses to build one.":
    "**`Self { field: expr, .. }` 字面量是类私有的构造方式** —— "
    "在类体内的任何地方都合法（类方法和实例方法皆可）；在体内也可以用类名来"
    "写（`Rect { .. }`）。字面量必须初始化每个没有初始化式的字段——字段全"
    "覆盖法则，结构体与类一视同仁——把一个字段写两遍也是错误；被省略的字段"
    "会运行其字段初始化式。私有字段可以在字面量中赋值 —— 但仅在类体内。这份"
    "隐私 _正是_ 密封性所在：外部代码只能通过调用一个愿意构建实例的类方法来构"
    "建类值。",

    # types.md — the new law paragraph.
    "A literal must cover **every** field — spelled in the literal, or "
    "covered by the field's declared initializer. Omitting a field with no "
    "initializer is a compile error, and so is spelling a field twice (see "
    "[structs](../reference/structs.md) for the exact diagnostics).":
    "字面量必须覆盖**每一个**字段——要么在字面量里写出，要么由该字段的声明"
    "初始化器覆盖。省略一个没有初始化器的字段是编译错误，把一个字段写两遍也"
    "是编译错误（确切的诊断信息见[结构体](../reference/structs.md)）。",

    # values-and-variables.md — the zero-value paragraph, second half
    # rewritten (the zero-fill claim is gone).
    "Every type has a zero value: `0` for numbers, `false` for `bool`, the "
    "empty string, `nil` for nullables. But nothing fills a struct literal's "
    "fields for you: a literal must cover every field — spelled in the "
    "literal, or covered by the field's declared initializer (see [structs, "
    "enums, and classes](types.md)).":
    "每个类型都有零值：数字是 `0`，`bool` 是 `false`，字符串是空串，可空类"
    "型是 `nil`。但没有任何东西会替你填充结构体字面量的字段：字面量必须覆盖"
    "每一个字段——要么在字面量里写出，要么由该字段的声明初始化器覆盖（参见"
    "[结构体、枚举与类](types.md)）。",

    # modules-and-visibility.md — the Uses paragraph, extended with the
    # unknown-import law (old translation's unchanged head carried over).
    "Importing a _module itself_ (`use tur_kit::layout;`) is an error — "
    "modules are namespaces, not values; the diagnostic names the fix (spell "
    "a name at the leaf). rut is fully statically typed: the compiler "
    "resolves every used name and knows from usage whether it lands in type "
    "position (`Column` in an annotation) or value position (`Logger(..)`), "
    "so there is nothing for the user to annotate. A use line may spell only "
    "what the target exports: a name the target doesn't declare is rejected "
    "at the use line itself — `` `ink` has no `Nope` — unknown import `` — "
    "not later, at a call site that might never be written. An unreferenced "
    "use name is a lint, not an error.":
    "导入_模块本身_（`use tur_kit::layout;`）是一个错误 —— 模块是命名空间，"
    "不是值；诊断会给出修复方法（在叶子处写出一个名字）。rut 是完全静态类型"
    "的：编译器解析每一个用到的名字，并根据用法判断它落在类型位置（注解中的 "
    "`Column`）还是值位置（`Logger(..)`），因此用户无需任何标注。use 行只能"
    "写出目标所导出的名字：目标没有声明的名字会在 use 行本身被拒绝——`` "
    "`ink` has no `Nope` — unknown import ``——而不是稍后在一个可能永远写不出"
    "来的调用点。未被引用的 use 名是一个 lint，不是错误。",

    # tutorial/modules.md — the use paragraph, extended with the law.
    "Builtin names — the primitives, `str`/`bytes` members, `panic`, "
    "`Vec`\\-free array grammar, `opaque` — are **ambient**: no `use` "
    "needed. The exceptions are core's import-gated names — the disposal "
    "context `DisposalContext` and the weak reference `Weak` — they resolve "
    "only through `use core::{ .. }`, like any package name. Package names "
    "from your manifest are imported the same way; an unused name in a `use` "
    "is a lint, not an error, while a name the target doesn't export is "
    "rejected at the use line itself — `` `ink` has no `Nope` — unknown "
    "import ``.":
    "内置名字——原语、`str`/`bytes` 成员、`panic`、`Vec`\\-free 数组文法、"
    "`opaque`——都是**环境自带**的：无需 `use`。例外是 core 的导入把守名字"
    "——disposal 上下文 `DisposalContext` 与弱引用 `Weak`——它们只能像任何包"
    "名一样经 `use core::{ .. }` 解析。清单点名的包名以同样的方式导入；`use` "
    "里未被使用的名字是 lint，不是错误，而目标没有导出的名字会在 use 行本身"
    "被拒绝——`` `ink` has no `Nope` — unknown import ``。",
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
