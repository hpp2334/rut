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

# Fresh translations for the strings this sweep changed (the book
# stops negating names it never had — pure-positive rewrites; the
# negating clauses are gone, so each fresh msgstr is the old zh with
# the foreign-name negation dropped). Terminology per
# docs/po/GLOSSARY.md; code spans byte-identical; the extracted
# markup style is mirrored (`_em_` emphasis, `**bold**`). Everything
# not in FRESH either keeps its msgstr byte-for-byte (unchanged
# msgid), rides MECH (the mechanical cutover shapes), or folds to ""
# — the English fallback BY DESIGN.
FRESH = {
    # ---- lanes 3+4 (the select + type-surface plan): `Future<T>` the
    # closed builtin class + the `async { }` block primitive, and the
    # bracket markers replacing the `builtin trait` rows. Terminology per
    # GLOSSARY: bracket marker → 括号标记, designated slot → 指定槽位,
    # mint → 铸造, async block → async 块, inject → 注入.
    # ---- lane 6 (the select + type-surface plan): structural
    # interfaces replace traits. Terminology per GLOSSARY:
    # interface → 接口, satisfy/satisfaction → 满足, member → 成员,
    # wrapper → 包装器, itable → itable 表, structural → 结构化的.







    # ---- lane 6: the newtype/wrapper + marker rows (the wrapper
    # families, the sealed-key law, the structural-satisfaction story) ----















































    # ---- the newtype landing (Lane 2 of the select + type-surface
    # plan): classes.md's "Newtypes: the one-field wrapper" section and
    # the lexical-structure construction-bullet amendment. Terminology
    # per GLOSSARY: newtype → 新类型, wrapper → 包装器, binder → 绑定名.









}  # the newtype sweep's strings; the select/completer entries below are
# carried byte-for-byte by the fold (unchanged msgids).

# ---- the async pair's rename: the rows pkg is `async_host` (was
# `async_engine`), the typed lib is `futures` (was `async_host`). Each
# fresh msgstr is the old zh with exactly the token swaps the English
# diff made — `rut_std::async_host::pkg()` and `async_host::pkg()`
# (the Rust builder) keep their spelling; the code-block msgids fold
# to "" by doctrine (they were untranslated before the rename too).
FRESH.update({
    'Both `launch_future` and `sleep` come from the `futures` package:':
        '`launch_future` 和 `sleep` 都来自 `futures` 包：',
    'Launchers are **host surface** — ordinary rut code over the closed `Future` '
    'class, provided by the `rut/futures` package (users may write their own the '
    'same way):':
        '启动器是**宿主面**——封闭 `Future` 类之上的普通 rut 代码，由 `rut/futures` '
        '包提供（用户可以同样方式编写自己的启动器）：',
    'Each embedder mounts `rut/async_host` (the rows `__launch`, `__abort`, '
    '`__sleep`, `__sleep_yield`) plus `rut/futures`, and installs the row bodies. '
    'A session that mounts neither simply has no launcher; `await` still works '
    'inline.':
        '每个嵌入方都挂载 `rut/async_host`（`__launch`、`__abort`、`__sleep`、'
        '`__sleep_yield` 这几行）外加 `rut/futures`，并安装这些行的函数体。'
        '哪一样都不挂载的会话就是没有启动器；`await` 仍可内联工作。',
    '// async_host + futures (optional)\n':
        '// async_host + futures (optional)\n',
    'mount `async_host` + `futures`':
        '挂载 `async_host` + `futures`',
    '`async_host` / `futures`':
        '`async_host` / `futures`',
    '`async_host` declares the engine rows (`__launch`, `__abort`, `__sleep`, '
    '`__sleep_yield`); `futures` restores the typed surface: `launch_future(f: '
    'Future<T>) -> LaunchedFutureHandle<T>`, `LaunchedFutureHandle.abort() -> '
    'bool`, `sleep(ms: u32) -> Future<nil>`. Each embedder mounts the pair '
    '**and** installs `rut_std::async_host::pkg()`; a session that mounts neither '
    'has no launcher ([launched futures](launched-futures.md), [host '
    'futures](host-futures.md)).':
        '`async_host` 声明引擎行（`__launch`、`__abort`、`__sleep`、`__sleep_yield`）；'
        '`futures` 还原类型化面：`launch_future(f: Future<T>) -> '
        'LaunchedFutureHandle<T>`、`LaunchedFutureHandle.abort() -> bool`、'
        '`sleep(ms: u32) -> Future<nil>`。每个嵌入方都挂载这对**并**安装 '
        '`rut_std::async_host::pkg()`；两者皆未挂载的会话没有启动器'
        '（[已启动的 future](launched-futures.md)、[宿主 future](host-futures.md)）。',
    '`inline = true` packages (ink, pouch, nmapset, json, strbuild, futures) are '
    'source-inlined into each consumer — required for class-method surfaces, '
    'whose inherent impls cross no module link boundary yet ([the '
    'frontend](frontend.md)).':
        '`inline = true` 的包（ink、pouch、nmapset、json、strbuild、futures）'
        '以源码形式内联进每个消费方——类方法表面需要如此，其固有 impl 尚不能跨模块'
        '链接边界（[前端](frontend.md)）。',
    'That leaves `inline = true` for packages whose methods live on class bodies '
    '(inherent impls cross no surface yet): the graph compiler splices their '
    'source into every consumer instead of linking them (`ink`, `json`, '
    '`nmapset`, `strbuild`, `futures`, `http`, `pouch`). See [Project structure '
    'and rut.jsonc](project-structure.md).':
        '于是 `inline = true` 留给了那些方法写在类体里的包（固有 impl 尚不能跨表面）：'
        '图编译器把它们的源码拼接进每个消费方，而不是链接它们（`ink`、`json`、'
        '`nmapset`、`strbuild`、`futures`、`http`、`pouch`）。'
        '见[项目结构与 rut.jsonc](project-structure.md)。',
    '**What a compiled bundle serves.** Instantiation is owner-anchored: a '
    'compiled owner carries the generic instantiations its own pack closure '
    'spelled in its ledger — and, the generic-source riding law, a compiled pkg '
    'whose surface exports generics also rides the source that serves '
    'consumer-spelled shapes: at the consumer\'s link a request the ledger lacks '
    'lowers the ridden text in the consumer\'s session and compiles the '
    'monomorphized body under the declaring pkg\'s spec (one row program-wide, '
    'nothing persisted). So the CDN lane delivers **host surfaces** (v10 decl '
    'bundles), **concrete-class libs** (`http`, `ink`, `strbuild` — methods '
    'cross on the surface\'s inherent rows), **and the generic owners** '
    '(`pouch`, `nmapset`, `json`, `futures` — `Vec<Todo>`, `Map<K,V>`, '
    '`decodeJson<T>` compile at the link from the ridden source). A bundle that '
    'predates the riding refuses a consumer-spelled shape loudly and says so '
    '(re-pack it), and a bundle-mounted json names its pack-time dev closure in '
    'its ledger, so the consumer\'s closure must contain those names '
    '(`pouch`, `nmapset` beside `json` — the six-pin law).':
        '**一个编译的包能服务什么。** 实例化由 owner 锚定：编译的 owner 在其台账中'
        '承载其打包期闭包拼写的泛型实例化 —— 加上泛型源码承载法则，表面导出泛型的'
        '编译 pkg 还会在二进制旁承载服务消费者拼写形状的源码：在消费者的链接处，'
        '台账缺失的请求会在消费者的会话中降低被承载的文本，并以声明 pkg 的 spec '
        '编译单态化代码体（全程序一行，不持久化）。因此 CDN 通道交付**宿主表面**'
        '（v10 decl 包）、**具体类库**（`http`、`ink`、`strbuild` —— 方法在表面的'
        '固有行上跨越），**以及泛型 owner**（`pouch`、`nmapset`、`json`、`futures` '
        '—— `Vec<Todo>`、`Map<K,V>`、`decodeJson<T>` 在链接处从被承载的源码编译）。'
        '早于承载法则的包会大声拒绝消费者拼写的形状并说明缘由（重新打包它），'
        '而包装载的 json 在其台账中命名其打包期开发闭包，因此消费者的闭包必须包含'
        '那些名字（`json` 旁的 `pouch`、`nmapset` —— 六钉法则）。',
    '`ink_host`, `http_host`, `nmap_host`, `async_host`, `bench_cross`':
        '`ink_host`, `http_host`, `nmap_host`, `async_host`, `bench_cross`',
    '`ink`, `http`, `strbuild`, `futures`':
        '`ink`, `http`, `strbuild`, `futures`',
    '**generic owners deliver too** — `pouch`, `nmapset`, `json`, `futures` ride '
    'their entry + group source beside the binaries, so a consumer\'s '
    '`Vec<Todo>` or `decodeJson<Vec<Todo>>` compiles **at the consumer\'s '
    'link**: the ridden text lowers in the consumer\'s session, the '
    'monomorphized bodies register under the declaring pkg\'s spec (one row '
    'program-wide, identity by owner), and nothing persists (`.rutc` caches '
    'stay pack-time). A bundle that predates the riding — no source beside the '
    'binary — refuses a consumer-spelled shape loudly and says so: re-pack it. '
    'A bundle-mounted json also names its pack-time dev closure in its ledger, '
    'so the consumer\'s closure must contain those names (`pouch`, `nmapset` '
    'beside `json` — the six-pin law).':
        '**泛型 owner 也交付** —— `pouch`、`nmapset`、`json`、`futures` '
        '在二进制旁承载其入口 + 组源码，因此消费者的 `Vec<Todo>` 或 '
        '`decodeJson<Vec<Todo>>` 在**消费者的链接处**编译：被承载的文本在消费者的'
        '会话中降低，单态化代码体以声明 pkg 的 spec 注册（全程序一行，以 owner '
        '为身份），不持久化任何东西（`.rutc` 缓存保持打包期）。早于承载的包 —— '
        '二进制旁没有源码 —— 会大声拒绝消费者拼写的形状并说明缘由：重新打包它。'
        '包装载的 json 还在其台账中命名其打包期开发闭包，因此消费者的闭包必须包含'
        '那些名字（`json` 旁的 `pouch`、`nmapset` —— 六钉法则）。',
})




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
