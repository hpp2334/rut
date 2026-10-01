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
# markdown structure load-bearing).
FRESH = {
 "Writers emit 10 **only** for decl roots — every lib bundle stays byte-identical v9, and old readers refuse a v10 bundle loudly at the version gate.":
  "写入方**只**为声明根发出 10——每个 lib 捆绑包保持字节等同的 v9，旧读取器在版本门大声拒绝 v10 捆绑包。",

 "First entry in the zip; JSONC (the directory grammar: `//` and `/* */` comments and trailing commas are legal); the same manifest grammar the directory form uses, byte-for-byte the directory's manifest — which is what makes a bundle-shaped directory pack unchanged:":
  "zip 中的第一个条目；JSONC（目录文法：`//` 与 `/* */` 注释以及尾随逗号都合法）；与目录形态使用同一份清单文法，逐字节就是目录的清单——这正是让捆绑包形态的目录原样打包的原因：",

 "compiled, the manifest JSON (`rut.json`): `.rutc` (v17) + `.d.rut` per linkable pkg — generic exports included, their instantiations seeded into the binaries, and a generic-owning pkg's source riding beside its binary — mixed source groups for `inline` deps and host pkgs":
  "编译，JSON 清单（`rut.json`）：每个可链接 pkg 一份 `.rutc`（v17）+ `.d.rut`——泛型导出在内，其实例化被播种进二进制，拥有泛型的 pkg 的源码在其二进制旁承载——`inline` 依赖与宿主 pkg 的混合源码组",

 "refused — re-pack the directory (the pre-JSONC wire)":
  "拒绝——请重新打包该目录（JSONC 之前的线格式）",

 "decl, the manifest JSON (`rut.json`): a `type = \"host\"` root — the manifest + its `.d.rut` surface, single-package":
  "声明，JSON 清单（`rut.json`）：一个 `type = \"host\"` 根——清单 + 它的 `.d.rut` 表面，单包",

 "compiled, the manifest JSONC (`rut.jsonc`): the v7 layout under the JSONC cutover — the manifest file renamed, comments + trailing commas legal":
  "编译，JSONC 清单（`rut.jsonc`）：JSONC 切换下的 v7 布局——清单文件更名，注释 + 尾随逗号合法",

 "decl, the manifest JSONC (`rut.jsonc`): a `type = \"host\"` root — the manifest + its `.d.rut` surface, single-package":
  "声明，JSONC 清单（`rut.jsonc`）：一个 `type = \"host\"` 根——清单 + 它的 `.d.rut` 表面，单包",

 "The 8→9 bump rides the manifest's own RENAME (`rut.json` → `rut.jsonc`): the entry name is layout, so the number moves with it — an old reader must never silently misparse a file whose name it does not know. Each historical extension existed because the added files were _part of the package_: a bundle that dropped peer groups or multi-lib files would load base-only — semantically wrong. v7 replaced the source contract with the compiled one — one exception, the riding law above: a generic-owning compiled pkg's source rides beside its binary so consumer-spelled shapes stay servable. Source sharing as the general contract stays a directory (`rut run <dir>`), as it always was outside bundles.":
  "8→9 的跳变搭载在清单自身的更名上（`rut.json` → `rut.jsonc`）：条目名就是布局，所以数字随它移动——旧读取器绝不能静默误解析一个它不认识其名字的文件。每一代历史扩展都因新增文件是*包的一部分*而存在：一个丢掉 peer 组或多 lib 文件的包会只装载基座 —— 语义错误。v7 用编译契约替换了源码契约 —— 一条例外，即上面的承载法则：拥有泛型的编译 pkg 的源码在其二进制旁承载，使消费者拼写的形状保持可服务。作为一般契约的源码共享仍是目录（`rut run <dir>`），一如包外世界的惯例。",

 "refuse — not a rut bundle; a `rut.json` entry instead → \"predates wire 9 — re-pack the directory\"":
  "拒绝——不是 rut 捆绑包；取而代之的是 `rut.json` 条目 → \"早于 wire 9——请重新打包该目录\"",

 "`format_version` exactly 9 or 10":
  "`format_version` 恰为 9 或 10",

 "refuse — \"reads bundle format_version 9 (compiled) and 10 (decl) only\"; ≤8 adds \"the pre-JSONC wire is retired\"":
  "拒绝——\"此工具链只读捆绑包 format_version 9（编译）与 10（声明）\"；≤8 附加\"JSONC 之前的线格式已退役\"",

 "The manifest text is **JSONC** — `//` line comments, `/* */` block comments, and trailing commas are all legal — parsed by `serde_json` behind a syntax-stripping front stage: the comment and comma bytes become spaces before the parser sees them, so a syntax error keeps the parser's own wording under a `line N:` prefix that names the ORIGINAL file's line. The value laws are unchanged. Value errors are path-targeted (`deps.pouch: unknown key 'feats'`). Descriptors are key/value objects; unknown descriptor keys are path-targeted manifest errors. Duplicate keys are last-wins. The old `_`\\-prefixed prose lane (`\"_comment\"`) retired with the JSONC cutover: an `_`\\-key refuses loudly naming the fix — comments are the prose now.":
  "清单文本是 **JSONC**——`//` 行注释、`/* */` 块注释以及尾随逗号都合法——由 `serde_json` 在一个语法剥离前级之后解析：注释与逗号的字节在解析器看到之前变成空格，因此语法错误保留解析器自己的措辞，带有一个指出原始文件行的 `line N:` 前缀。值法则不变。值错误按路径定位（`deps.pouch: unknown key 'feats'`）。描述符是键/值对象；未知描述符键是按路径定位的清单错误。重复键后者胜。旧的 `_`\\-前缀散文车道（`\"_comment\"`）随 JSONC 切换退役：`_`\\-键大声拒绝并指出修复——注释才是散文。",

 "Anything else is refused at the door (exit 2): a loose `.rut` file is not a runnable unit — give the directory a `rut.jsonc` (`{\"name\": \"…\", \"entry\": {\"lib\": \"./<file>.rut\"}}` — JSONC: `//` comments and trailing commas are legal), or run a packed `.rutbundle`.":
  "其他任何输入都会在门口被拒绝（退出码 2）：散的 `.rut` 文件不是可运行单元——给目录一个 `rut.jsonc`（`{\"name\": \"…\", \"entry\": {\"lib\": \"./<file>.rut\"}}`——JSONC：`//` 注释与尾随逗号合法），或运行打包好的 `.rutbundle`。",
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
