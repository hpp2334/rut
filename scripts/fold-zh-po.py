#!/usr/bin/env python3
"""The po fold — merge the fresh rut.pot into docs/po/zh_CN.po.

Merge law (the GLOSSARY contract, after the 432ace9 lesson):
- entries in POT order; the header rides from the existing zh_CN.po;
- an UNCHANGED msgid keeps its msgstr byte-for-byte;
- a CHANGED msgid whose diff is exactly the mechanical cutover
  (rut.json → rut.jsonc, wire v7→v9 / v8→v10) carries the old msgstr
  through the SAME mechanical substitution — the zh text moves with
  the file name and the wire numbers, nothing else;
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
# markdown structure load-bearing). The vocabulary purge's zh shape
# rides MECH (数据类 → 结构体 alongside dataclass → struct); everything
# below is new or non-mechanically changed prose. The three new ```rut
# blocks fold to "" — the untranslated set is code-only, by doctrine.
FRESH = {
 "An enum is a distinct named type over integer constants. No payloads and no computed members — where another language would use a union of literal strings, rut uses an enum:":
  "枚举是建立在整型常量之上的一个独立命名类型。没有载荷，也没有计算型成员——换作别的语言会在字面量字符串的并集上做文章的地方，rut 用枚举：",

 "`when` over an enum must be exhaustive — every member, or an `else` arm (see [control flow and when](control-flow.md)). Enums render as their member name in format strings. Enums take `impl` blocks — non-self methods on the name, `self` methods on a value, and trait impls (`impl Iterator<E> for Light` makes `for (let v of l)` walk) — see [enums](../reference/enums.md).":
  "对枚举的 `when` 必须穷尽——覆盖每个成员，或带一个 `else` 分支（参见[控制流与 when](control-flow.md)）。枚举在格式化字符串里渲染为成员名。枚举可以带 `impl` 块——非 self 方法挂在枚举名上调用，`self` 方法挂在值上调用，还有 trait impl（`impl Iterator<E> for Light` 让 `for (let v of l)` 走起来）——参见[枚举](../reference/enums.md)。",

 "Declare with `struct`, construct with a literal — anywhere in a function body, nested inside other literals. Construction is the literal; there is no constructor gate:":
  "用 `struct` 声明，用字面量构造——函数体内的任何位置，或嵌在其他字面量里。构造就是字面量；没有构造器门禁：",

 "All fields are public, always — member visibility in a struct is a compile error. Privacy is what classes are for. Methods live in `impl` blocks with real visibility — `pub fn` exports cross-module, plain `fn` stays module-private (see [structs](../reference/structs.md)).":
  "所有字段永远公开——给结构体成员标注可见性是编译错误。私有正是类存在的意义。方法住在 `impl` 块里，可见性是真实的——`pub fn` 跨模块导出，不带 `pub` 的 `fn` 只在模块内可见（参见[结构体](../reference/structs.md)）。",

 "**Struct fields are always public** — the field law has no dial (see [Structs](structs.md)). **Impl-block methods take the `pub` dial** — `pub fn` exports cross-module, plain `fn` is module-private — the same law a class's methods follow, enums included (see [Structs](structs.md) and [Enums](enums.md)). Trait method signatures and impl methods are as visible as their trait (see [Traits and dispatch](traits.md)).":
  "**结构体的字段永远公开**——字段法则没有调节项（参见[结构体](structs.md)）。**impl 块的方法走 `pub` 调节项**——`pub fn` 跨模块导出，不带 `pub` 的 `fn` 只在模块内可见——与类的方法同一法则，枚举亦然（参见[结构体](structs.md)与[枚举](enums.md)）。trait 方法签名与 impl 方法的可见性与其 trait 一致（参见[trait 与分派](traits.md)）。",

 "No data payloads and no computed members — ever. Methods are an `impl` block away (see [Impl blocks](#impl-blocks)).":
  "没有数据载荷，也没有计算型成员——永远如此。方法就在一个 `impl` 块之外（参见[impl 块](#impl-blocks)）。",

 "Enums take `impl` blocks — the same three forms a struct or class takes: non-self methods (called on the enum's name), `self` methods (called on a value), and trait impls.":
  "枚举可以带 `impl` 块——与结构体和类相同的三种形式：非 self 方法（通过枚举名调用）、`self` 方法（通过值调用），以及 trait impl。",

 "A non-self method is a namespaced function: `Dial.default()`. `Self` spells the enum, so `-> Self` is the return type and `Dial.Low` mints the member.":
  "非 self 方法是一个带命名空间前缀的函数：`Dial.default()`。`Self` 拼写的就是这个枚举，所以 `-> Self` 是返回类型，`Dial.Low` 铸造出成员。",

 "A `self` method dispatches on the value: `d.flipped()` — the receiver crosses as the singleton cell it already is.":
  "`self` 方法在值上分派：`d.flipped()`——接收者本来就是单例单元，按原样跨越。",

 "Method visibility is real: `pub fn` exports cross-module, plain `fn` is module-private (see [Modules and visibility](modules-and-visibility.md)).":
  "方法可见性是真实的：`pub fn` 跨模块导出，不带 `pub` 的 `fn` 只在模块内可见（参见[模块与可见性](modules-and-visibility.md)）。",

 "Trait impls make enum values iterable — `for (let v of c)` rides the same desugar as a class's (see [The iteration protocol](traits.md)):":
  "trait impl 让枚举值可迭代——`for (let v of c)` 走与类相同的脱糖（参见[迭代协议](traits.md)）：",

 "One limit stays: `Disposal` is for structs and classes only — the engine disposes a record's cell, and an enum member is immortal.":
  "只有一条限制仍在：`Disposal` 只属于结构体和类——引擎处置的是记录的单元，而枚举成员是不朽的。",

 "Iterating form — vecs, fixed arrays, slices, strings (one-codepoint `str`s per step), `bytes` (`u8` per step), and any type with a registered `Iterator` impl — an enum value included (see [Traits and dispatch](traits.md) and [Enums](enums.md)):":
  "迭代形式 —— vec、定长数组、切片、字符串（每步一个单码点 `str`）、`bytes`（每步一个 `u8`），以及任何注册了 `Iterator` impl 的类型——枚举值也在内（参见[trait 与分派](traits.md)与[枚举](enums.md)）：",

 "Methods carry real visibility — the class rule: `pub fn` exports cross-module, plain `fn` is module-private. `Self` spells the struct, in signatures (`-> Self`) and the literal (`Self { x: 1, y: 2 }`) alike:":
  "方法带有真实可见性——类的法则：`pub fn` 跨模块导出，不带 `pub` 的 `fn` 只在模块内可见。`Self` 拼写的就是这个结构体，签名（`-> Self`）和字面量（`Self { x: 1, y: 2 }`）里都一样：",

 "**fields are always public** — no field-visibility dial (privacy needs construction control, which is the class's job — see [Classes and constructors](classes.md));":
  "**字段永远公开**——字段可见性没有调节项（私有需要构造控制，那是类的职责——参见[类与构造器](classes.md)）；",

 "**construction is the literal** — `Point { x: 1, y: 2 }` everywhere (open literal vs class-method-gated _is_ the struct/class distinction).":
  "**构造就是字面量**——`Point { x: 1, y: 2 }` 到处可用（开放字面量 vs 类方法门禁_正是_结构体/类的分界）。",

 "Everything else class-shaped is allowed — `impl` blocks, `Self`, `Disposal`. And the old \"no destructor\" limit is gone: a shared value dies exactly when its cell's refcount reaches zero, so cleanup is one `impl` away — implement `Disposal` for the type and the engine calls `dispose` at that moment (see [Rc, dispose, and identity](rc-dispose-identity.md)).":
  "其余类形态的东西都允许——`impl` 块、`Self`、`Disposal`。而旧的“没有析构器”限制已经消失：共享值恰在其单元引用计数归零时死亡，所以清理只是一个 `impl` 之遥——为该类型实现 `Disposal`，引擎便会在那一刻调用 `dispose`（参见[Rc、dispose 与同一性](rc-dispose-identity.md)）。",

 "An enum value iterates the same way: `impl Iterator<E> for Color` makes `for (let v of c)` walk whatever the impl's `iterate` emits — the desugar is the trait, the target's kind is irrelevant (see [Enums](enums.md)).":
  "枚举值以同样的方式迭代：`impl Iterator<E> for Color` 让 `for (let v of c)` 走遍 impl 的 `iterate` 所发射的一切——脱糖的是 trait，目标的种类无关紧要（参见[枚举](enums.md)）。",

 "A user record is an **array of 8-byte slots**:":
  "用户记录是一个 **8 字节槽位的数组**：",

 "user record (struct)":
  "用户记录（struct）",

 "One flat `Tok` enum for literals, punctuation, and operators. Keywords are ordinary `Ident`s; the parser matches them by interner name. **Reserved words are rejected by the lexer** with a message naming the rut replacement (`interface`, `private`, `void`, `in`); `?.` and `??` are punctuation-level rejections, a separate mechanism.":
  "一个扁平的 `Tok` 枚举覆盖字面量、标点和运算符。关键字就是普通的 `Ident`；语法分析器按驻留名匹配它们。**保留字由词法分析器拒绝**，消息会指明 rut 的替代写法（`interface`、`private`、`void`、`in`）；`?.` 与 `??` 是标点层面的拒绝，属于另一套机制。",
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
