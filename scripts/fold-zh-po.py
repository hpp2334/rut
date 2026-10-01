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
 "The binary embeds the whole engine: lexer, parser, compiler, typed bytecode, and the VM. The toolchain's standard packages (`core`, `pouch`, `ink`, `json`, …) ship as compiled `.rutbundle`s on jsDelivr — a program says `use ink::{ Logger };` and its manifest's `deps` row mounts the package from the CDN, pinned by sha256 ([dependency kinds](../reference/dependency-kinds.md)), since every rut program is a module directory ([your first rut program](first-program.md)).":
  "这个二进制内嵌了整个引擎：词法分析器、语法分析器、编译器、类型化字节码和 VM。工具链的标准包（`core`、`pouch`、`ink`、`json` 等）以编译好的 `.rutbundle` 交付在 jsDelivr 上——程序写一行 `use ink::{ Logger };`，清单的 `deps` 行就从 CDN 挂载这个包，以 sha256 固定（[依赖种类](../reference/dependency-kinds.md)），因为每个 rut 程序都是一个模块目录（[你的第一个 rut 程序](first-program.md)）。",

 "The first run fetched `ink` from jsDelivr into `hello/.rut/cache`; `rut fetch hello` pre-warms that cache without running anything.":
  "第一次运行把 `ink` 从 jsDelivr 拉取进 `hello/.rut/cache`；`rut fetch hello` 可以在不运行任何东西的情况下预热该缓存。",

 "The run works from ANY directory — nothing about it assumes a clone of the toolchain's repo. The url row names the package on jsDelivr; the CLI fetches it once into the project's cache (`.rut/cache`), every later run is a pure cache hit, and `rut fetch hello` pre-warms the cache without running anything.":
  "这一运行在任何目录下都能工作——它不假设你克隆了工具链的仓库。url 行写明包在 jsDelivr 上的位置；CLI 把它拉取一次进项目缓存（`.rut/cache`），之后的每次运行都是纯缓存命中，而 `rut fetch hello` 可以在不运行任何东西的情况下预热缓存。",

 "The app names its dependencies in `deps`, by url or path — each package pulls its own dependencies along (`ink` brings the host surface `ink_host`; you never spell it):":
  "应用在 `deps` 里写明依赖，按 url 或路径——每个包把自己的依赖一并带入（`ink` 会带上宿主面 `ink_host`；你永远不用手写它）：",

 "**Which kind, when.** The toolchain's standard packages (`core`, `ink`, `pouch`, `json`, …) are consumed as **pinned url rows** from this CDN — that is the recommended import for everything the toolchain ships. A `path` row is for **your own local packages**: a sibling directory in the same project (the modules tutorial's `greet` app mounting `../pkg` is the shape). The tag advances with format changes (`std-v3` today) and is never re-pointed, so a pin at a tag stays honest forever.":
  "**哪种关系，何时用。**工具链的标准包（`core`、`ink`、`pouch`、`json` 等）都作为来自这个 CDN 的**固定 url 行**来消费——这是工具链交付的一切内容的推荐导入方式。`path` 行用于**你自己的本地包**：同一项目里的一个兄弟目录（模块教程里 `greet` 应用挂载 `../pkg` 就是这个形状）。标签随格式变化前进（今天是 `std-v3`）且永不重指，所以钉在标签上的固定值永不腐烂。",

 "The std tree ships as committed per-package bundles — `https://cdn.jsdelivr.net/gh/hpp2334/rut@<tag>/dist/std/<pkg>.rutbundle`, pinned by sha256 (the shapes here keep placeholder tags; the worked manifests in the quick-start, the tutorial, and the examples carry the real pins — a pin at an immutable tag cannot rot). Tags advance (`std-vNN`) and are never re-pointed — jsDelivr caches aggressively; the artifacts are committed at `dist/std/`, packed by `scripts/pack-std.cjs` (whose `--check` gate is a pure byte-equality repack — CI never touches the network).":
  "std 树以已提交的按包捆绑包交付——`https://cdn.jsdelivr.net/gh/hpp2334/rut@<tag>/dist/std/<pkg>.rutbundle`，以 sha256 固定（此处的形状保留占位标签；快速开始、教程与示例里的实际清单携带真钉——钉在不可变标签上的固定值永不腐烂）。标签只前进（`std-vNN`），永不重指——jsDelivr 缓存很凶；工件提交在 `dist/std/`，由 `scripts/pack-std.cjs` 打包（其 `--check` 门是纯字节等值的重打包——CI 永不触网）。",
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
