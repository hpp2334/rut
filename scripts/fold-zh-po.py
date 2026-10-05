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

# Fresh translations for the strings this sweep changed (the one-wire
# bundles rewrite: `format_version` is 10 always and `type` routes
# lib-compiled vs host-decl; the module binary states VERSION 21 once;
# the riding law stated in the present tense with the mount refusal —
# "`pouch` owns an open generic surface but its bundle carries no
# riding source — re-pack the directory"; the 9 pin pages move to
# `std-v8`; the `[constructor]` call-form mirrors lose the `.new()`
# equivalence clauses). Terminology per docs/po/GLOSSARY.md (捆绑包 /
# 编译根 / 声明根 / 承载法则 / 作用域台账 / 固定值 / 指定表面); code
# spans and diagnostics byte-identical; URLs/shas are code spans; the
# extracted markup style is mirrored (`**bold**`, `_emphasis_`,
# full-width punctuation in prose). Everything not in FRESH either
# keeps its msgstr byte-for-byte (unchanged msgid), rides MECH (the
# mechanical cutover shapes), or folds to "" — the English fallback BY
# DESIGN. Code blocks fold to "" (the untranslated set is code-only).
# FRESH is pruned after every landed sweep: once a msgstr is folded
# into zh_CN.po it is kept byte-for-byte by the fold itself, so the
# dict only ever carries translations for the CURRENT sweep's new and
# changed strings. The phase-2 sweep's `[constructor]`-designation
# entries landed and were pruned; see the git history of this file.
FRESH = {
    '`rut pack mod/` reads a module directory and emits **one file** — `mod.rutbundle` — a zip archive carrying the module in one of two root kinds, routed by the manifest\'s `type` key: a **lib** pkg packs **compiled** (the root\'s `.rutc` binary, its scope ledger, and each dependency as a compiled `.rutc` group or a source file set), and a `type = "host"` pkg packs as a **decl** root (its `.d.rut` surface rides as source; single-package). A bundle is the same contract as the directory it was packed from, in one file: the loader mounts it exactly like the directory, and the two compile to identical programs.':
        '`rut pack mod/` 读入模块目录并产出**一个文件**——`mod.rutbundle`——一个 zip 档案，以两种根之一携带模块，由清单的 `type` 键路由：**lib** 包**编译**打包（根的 `.rutc` 二进制、其作用域台账、每个依赖作为编译 `.rutc` 组或源文件集），`type = "host"` 包打包为**声明**根（其 `.d.rut` 表面以源码承载；单包）。捆绑包与打包前的目录是同一契约，收进一个文件：加载器挂载它与挂载目录完全一样，两者编译出相同的程序。',
    '**Entry names are paths relative to the module root** — `rut.jsonc`, `<pkg>.rutc`, `<pkg>.d.rut`, `<dep>/…` group entries for deps. No directories otherwise, no metadata entries. Unknown extra entries are ignored by loaders (forward compatibility) — except in a decl bundle, where a group entry, a root `.rutc`, or a ledger is a contradiction and refuses (see [the decl root](#the-decl-root-host)).':
        '**条目名是相对模块根的路径**——`rut.jsonc`、`<pkg>.rutc`、`<pkg>.d.rut`、依赖的 `<dep>/…` 组条目。除此之外没有目录，也没有元数据条目。未知的额外条目被加载器忽略（向前兼容）——声明捆绑包除外：组条目、根 `.rutc` 或台账在那里是矛盾，会拒绝（见[声明根](#the-decl-root-host)）。',
    'The compiled root (lib)':
        '编译根（lib）',
    "**Generics ride compiled — and their source rides beside them** (the generic-source riding law): instantiation is owned by the declaring package, and the pack walk seeds each generic dep with the instantiations its consumers spell — the binary's instantiation ledger names every `(owner, decl, arguments)` row. A compiled pkg whose surface exports an OPEN generic surface (generic fns, generic type exports, generic methods, generic-target impls) ALSO rides the source that serves consumer-spelled shapes: the entry lib, each `entry.libs` file, and each `peer-deps` group file, verbatim, beside the binary. The entry lib's presence is the loader's dispatch marker; non-generic pkgs (`http`, `ink`, `strbuild`) stay source-free by law. At the consumer's link, a request the ledger lacks lowers the ridden text in the consumer's session and compiles the monomorphized body with owner = the pkg's spec — one row program-wide, indistinguishable from a pack-time one, nothing persisted. The law binds both sides: a pkg that owns an open generic surface but whose bundle carries no riding source refuses at the mount (`pouch` owns an open generic surface but its bundle carries no riding source — re-pack the directory), never mislinks. `--strip` refuses the combination: the ridden text would recompile clean-named beside mangled binaries.":
        '**泛型编译承载——其源码在二进制旁承载**（泛型源码承载法则）：实例化归声明包所有，打包遍历以消费者拼写的实例化为每个泛型依赖播种——二进制的实例化台账命名每一行 `(owner, decl, arguments)`。表面导出开放泛型表面（泛型 fn、泛型类型导出、泛型方法、泛型目标的 impl）的编译 pkg **还会**在二进制旁逐字节承载服务消费者拼写形状的源码：入口 lib、每个 `entry.libs` 文件、每个 `peer-deps` 组文件。入口 lib 的存在是加载器的分派标记；非泛型包（`http`、`ink`、`strbuild`）依法保持无源码。在消费者的链接处，台账没有的请求会降低消费者会话中被承载的文本，并以 owner = 该包的 spec 编译单态化代码体——全程序一行，与打包期的行无从区分，不持久化。法则约束两侧：拥有开放泛型表面却不带承载源码的包在挂载时拒绝（`pouch` owns an open generic surface but its bundle carries no riding source — re-pack the directory），绝不错链。`--strip` 拒绝这一组合：被承载的文本会在改名后的二进制旁以干净名字重编译。',
    '**The root must be linkable** — else `pack: <pkg> is inline — its source is its interface and it cannot be published compiled; share the directory instead`. A root that has nothing to compile is not this refusal: a `type = "host"` pkg packs as the decl root, below.':
        '**根必须可链接**——否则 `pack: <pkg> is inline — its source is its interface and it cannot be published compiled; share the directory instead`。没有东西可编译的根不是这条拒绝：`type = "host"` 包按声明根打包，见下。',
    'The decl root (host)':
        '声明根（host）',
    "`rut pack <host-dir>` works (the surface verifies by parsing and lowering once — the exact lane a mount runs — then rides as source), `--strip` refuses on a host root (`no symbols to strip` — no programs, no sidecar), and `rut run <host.rutbundle>` refuses with intent: a host bundle carries a declaration surface — nothing to run; bind its rows from the embedder ([host fns](host-fns.md)). The mount is the same lane a host group rides inside a compiled bundle: the surface lowers into the pkg's host rows.":
        '`rut pack <host-dir>` 可行（表面经解析与降低各一次来验证——挂载运行的同一条车道——然后以源码承载），`--strip` 在宿主根上拒绝（`no symbols to strip`——没有程序，没有旁车），`rut run <host.rutbundle>` 带着意图拒绝：宿主捆绑包携带的是声明表面——没有可运行的东西；从嵌入方绑定它的行（[宿主函数](host-fns.md)）。挂载与编译捆绑包内的宿主组走的是同一条车道：表面降低为该包的宿主行。',
    "Because the manifest rides byte-for-byte, url rows carry into a consumer's own `rut pack` output, satisfied by the rode-along groups: the archive's groups re-encode from the consumer session's units (one `.rutc` path), source groups copy their file set under the new prefix, and the same manifest + pins still pack byte-identically. The pack refuses, loudly and named, the cases that would emit a bundle the loader must reject: an archive source group still declaring `dev-deps` directories (vendor the dep), a compiled group the consumer's closure never uses (it cannot ride unpackaged), and a declared dep that did not ride.":
        '由于清单逐字节随行，url 行会带进消费者自己的 `rut pack` 输出，由随行组满足：存档的组从消费者会话的 units 重新编码（唯一的 `.rutc` 路径），源码组把它的文件集复制到新前缀下，而相同清单 + 相同固定值仍逐字节相同地打包。打包会响亮地、指名道姓地拒绝那些会产出加载器必须拒绝的捆绑包的情形：仍在声明 `dev-deps` 目录的存档源码组（vendor 这个依赖）、消费者闭包从不使用的编译组（它无法未打包地承载），以及没有承载的已声明依赖。',
    'The wire version':
        '线上版本',
    "`format_version` is 10 — the one bundle wire, both root kinds; the manifest's `type` routes lib-compiled vs host-decl. A loader refuses any other value **before reading anything else** — refuse, never guess: `this toolchain reads bundle format_version 10 only (found {v}) — re-pack the directory`.":
        '`format_version` 是 10——唯一的捆绑包线上格式，两种根皆然；清单的 `type` 路由 lib-编译与宿主-声明两种根。加载器**在读取其他任何东西之前**拒绝其他任何值——拒绝，绝不猜测：`this toolchain reads bundle format_version 10 only (found {v}) — re-pack the directory`。',
    'Every file in the two layouts is part of the package: a bundle that dropped peer groups or multi-lib files would load base-only — semantically wrong, so the closure law is checked at the mount. Compilation is the delivery contract — one exception, the riding law above: a pkg with an open generic surface rides its source beside the binary so consumer-spelled shapes stay servable. Source sharing as the general contract stays a directory (`rut run <dir>`), as it always was outside bundles.':
        '两种布局里的每个文件都是包的一部分：丢掉对等组或多 lib 文件的捆绑包会只装基础部分——语义上是错的，所以闭包法则在挂载时校验。编译是交付契约——一个例外，即上文的承载法则：拥有开放泛型表面的包在二进制旁承载自己的源码，消费者拼写的形状因此始终可服务。作为一般契约的源码共享仍是目录（`rut run <dir>`），在捆绑包之外一如既往。',
    "Peer-gated packages publish inside a compiled bundle two ways, per their group-kind law: a NON-generic compiled declarer's peer groups compile into its binary at pack time (the rows ride the binary; the group files do not travel), and a splice-needed declarer publishes as a source group, its group files riding beside the entry — the loader's peer gate appends by presence exactly as in a directory world ([Dependency kinds](dependency-kinds.md)). A generic-owning compiled declarer rides its group files too (the riding law), and the on-demand recompile splices exactly the rows whose peers are in the consumer's closure.":
        '对等门控的包在编译捆绑包内以两种方式发布，依其组种类法则：非泛型的编译声明者，其对等组在打包期编译进它的二进制（行随二进制承载；组文件不旅行），需要拼接的声明者则以源码组发布，其组文件在入口旁承载——加载器的对等门按在场与否追加，与目录世界完全一样（[依赖种类](dependency-kinds.md)）。拥有泛型的编译声明者也承载自己的组文件（承载法则），按需重编译恰好拼接对等者在消费者闭包中的那些行。',
    'refuse — not a rut bundle':
        '拒绝——不是 rut 捆绑包',
    '`format_version` exactly 10':
        '`format_version` 恰为 10',
    'refuse — `this toolchain reads bundle format_version 10 only (found {v}) — re-pack the directory`':
        '拒绝——`this toolchain reads bundle format_version 10 only (found {v}) — re-pack the directory`',
    'a host root carrying a root `.rutc`':
        '宿主根携带根 `.rutc`',
    "refuse — a host bundle's root is its surface, not a compiled unit":
        '拒绝——宿主捆绑包的根是它的表面，不是编译单元',
    'a host root carrying a `rut.scopes` ledger or a group entry':
        '宿主根携带 `rut.scopes` 台账或组条目',
    'a compiled root without its `rut.scopes` ledger, or a ledger row that is not a bare package name':
        '编译根没有自己的 `rut.scopes` 台账，或台账行不是裸包名',
    'refuse — a corrupt compiled bundle':
        '拒绝——损坏的编译捆绑包',
    'every declared dep satisfied by a group (compiled roots)':
        '每个已声明依赖都被一个组满足（编译根）',
    "**What a compiled bundle serves.** Instantiation is owner-anchored: a compiled owner carries the generic instantiations its own pack closure spelled in its ledger — and, the generic-source riding law, a compiled pkg whose surface exports generics also rides the source that serves consumer-spelled shapes: at the consumer's link a request the ledger lacks lowers the ridden text in the consumer's session and compiles the monomorphized body under the declaring pkg's spec (one row program-wide, nothing persisted). So the CDN lane delivers **host surfaces** (decl bundles), **concrete-class libs** (`http`, `ink`, `strbuild` — methods cross on the surface's inherent rows), **and the generic owners** (`pouch`, `nmapset`, `json`, `futures` — `Vec<Todo>`, `Map<K,V>`, `decodeJson<T>` compile at the link from the ridden source). The riding law binds this lane too: a pkg that owns an open generic surface but carries no riding source refuses at the mount (`pouch` owns an open generic surface but its bundle carries no riding source — re-pack the directory), and a bundle-mounted json names its pack-time dev closure in its ledger, so the consumer's closure must contain those names (`pouch`, `nmapset` beside `json` — the six-pin law).":
        '**编译捆绑包服务什么。**实例化以属主为锚：编译的属主在其台账中承载自己的打包闭包拼写的泛型实例化——而且，按泛型源码承载法则，表面导出泛型的编译 pkg 还在二进制旁承载服务消费者拼写形状的源码：在消费者的链接处，台账没有的请求会在消费者的会话中降低被承载的文本，并以声明 pkg 的 spec 编译单态化代码体（全程序一行，不持久化）。所以 CDN 车道交付**宿主表面**（声明捆绑包）、**具体类库**（`http`、`ink`、`strbuild`——方法在表面的固有行上跨越）、**以及泛型 owner**（`pouch`、`nmapset`、`json`、`futures`——`Vec<Todo>`、`Map<K,V>`、`decodeJson<T>` 在链接处从被承载的源码编译）。承载法则也约束这条车道：拥有开放泛型表面却不带承载源码的包在挂载时拒绝（`pouch` owns an open generic surface but its bundle carries no riding source — re-pack the directory），包装载的 json 还在其台账中命名其打包期开发闭包，因此消费者的闭包必须包含那些名字（`json` 旁的 `pouch`、`nmapset`——六钉法则）。',
    "The manifest text is **JSONC** — `//` line comments, `/* */` block comments, and trailing commas are all legal — parsed by `serde_json` behind a syntax-stripping front stage: the comment and comma bytes become spaces before the parser sees them, so a syntax error keeps the parser's own wording under a `line N:` prefix that names the ORIGINAL file's line. The value laws are unchanged. Value errors are path-targeted (`deps.pouch: unknown key 'feats'`). Descriptors are key/value objects; unknown descriptor keys are path-targeted manifest errors. Duplicate keys are last-wins. An `_`\\-key refuses loudly naming the fix — comments are the prose now.":
        "清单文本是 **JSONC**——`//` 行注释、`/* */` 块注释以及尾随逗号都合法——由 `serde_json` 在一个语法剥离前级之后解析：注释与逗号的字节在解析器看到之前变成空格，因此语法错误保留解析器自己的措辞，带有一个指出原始文件行的 `line N:` 前缀。值法则不变。值错误按路径定位（`deps.pouch: unknown key 'feats'`）。描述符是键/值对象；未知描述符键是按路径定位的清单错误。重复键后者胜。`_`\\-键大声拒绝并指出修复——注释才是散文。",
    "**Which kind, when.** The toolchain's standard packages (`core`, `ink`, `pouch`, `json`, …) are consumed as **pinned url rows** from this CDN — that is the recommended import for everything the toolchain ships. A `path` row is for **your own local packages**: a sibling directory in the same project (the modules tutorial's `greet` app mounting `../pkg` is the shape). The tag advances with format changes (`std-v8` today) and is never re-pointed, so a pin at a tag stays honest forever.":
        '**哪种关系，何时用。**工具链的标准包（`core`、`ink`、`pouch`、`json` 等）都作为来自这个 CDN 的**固定 url 行**来消费——这是工具链交付的一切内容的推荐导入方式。`path` 行用于**你自己的本地包**：同一项目里的一个兄弟目录（模块教程里 `greet` 应用挂载 `../pkg` 就是这个形状）。标签随格式变化前进（今天是 `std-v8`）且永不重指，所以钉在标签上的固定值永不腐烂。',
    "**What a url row can deliver** is the engine's instantiation law, read from the CDN side: a compiled bundle carries the generic instantiations its own pack closure spelled **in its ledger**, and — by the generic-source riding law — a compiled pkg whose surface exports generics **also rides the source that serves consumer-spelled shapes**:":
        '**一条 url 行能交付什么**是从 CDN 一侧读到的引擎实例化法则：编译的包在其台账中承载其打包期闭包拼写的泛型实例化，而且——按泛型源码承载法则——表面导出泛型的编译 pkg **还会承载服务消费者拼写形状的源码**：',
    "**generic owners deliver too** — `pouch`, `nmapset`, `json`, `futures` ride their entry + group source beside the binaries, so a consumer's `Vec<Todo>` or `decodeJson<Vec<Todo>>` compiles **at the consumer's link**: the ridden text lowers in the consumer's session, the monomorphized bodies register under the declaring pkg's spec (one row program-wide, identity by owner), and nothing persists (`.rutc` caches stay pack-time). The riding law binds both sides: a pkg that owns an open generic surface but carries no riding source refuses at the mount (`pouch` owns an open generic surface but its bundle carries no riding source — re-pack the directory). A bundle-mounted json also names its pack-time dev closure in its ledger, so the consumer's closure must contain those names (`pouch`, `nmapset` beside `json` — the six-pin law).":
        '**泛型 owner 也交付** —— `pouch`、`nmapset`、`json`、`futures` 在二进制旁承载其入口 + 组源码，因此消费者的 `Vec<Todo>` 或 `decodeJson<Vec<Todo>>` 在**消费者的链接处**编译：被承载的文本在消费者的会话中降低，单态化代码体以声明 pkg 的 spec 注册（全程序一行，以 owner 为身份），不持久化任何东西（`.rutc` 缓存保持打包期）。承载法则约束两侧：拥有开放泛型表面却不带承载源码的包在挂载时拒绝（`pouch` owns an open generic surface but its bundle carries no riding source — re-pack the directory）。包装载的 json 还在其台账中命名其打包期开发闭包，因此消费者的闭包必须包含那些名字（`json` 旁的 `pouch`、`nmapset` —— 六钉法则）。',
    'Packs a module directory into a **deterministic** `.rutbundle` — same input, same bytes. Without `-o`, the output is written beside the input as `<dir-name>.rutbundle`. A **lib** pkg packs **compiled** (format_version 10): the root and every linkable package ride as `.rutc` binaries (bodies + surface — the linking truth), splice-needed packages (generic exports, interface-typed parameters, `inline`) and host pkgs ride as source groups, and a scope ledger lets any loader rebase the binaries onto its own numbering. A root that cannot link is refused — share the directory instead. A **`type = "host"` pkg packs as a decl root** (its `.d.rut` surface riding as source, single-package), `--strip` refuses there (`no symbols to strip`), and `run` accepts either bundle directly — though running a host bundle refuses with intent: bind its rows from the embedder ([module bundles](bundles.md)).':
        '把模块目录打包成**确定性的** `.rutbundle`——相同输入，相同字节。没有 `-o` 时，输出写在输入旁边，名为 `<dir-name>.rutbundle`。**lib** 包**编译**打包（format_version 10）：根与每个可链接的包都以 `.rutc` 二进制承载（函数体 + 表面——链接的真相），需要拼接的包（泛型导出、接口类型参数、`inline`）与宿主包则以源码组承载，作用域台账让任何加载器把二进制重定基到自己的编号上。无法链接的根会被拒绝——改为共享目录。**`type = "host"` 包按声明根打包**（其 `.d.rut` 表面以源码承载，单包），`--strip` 在那里拒绝（`no symbols to strip`），而 `run` 直接接受两种捆绑包——不过运行宿主捆绑包会带着意图拒绝：从嵌入方绑定它的行（[模块捆绑包](bundles.md)）。',
    "Both root kinds load (a compiled root with its groups and its pack-time scope ledger, rebased at the close of the world; a decl root as the pkg's host rows), first-mount-wins, and the bundle root's package name is `loaded.root`. Wasm hosts `include_bytes!` the committed artifact and parse through `Pkg::from_bundle`. For the url-dep _walk_ (pins, closure checks, the peer gate over archive groups) stay on the `load_dir*` lanes — a bundle mount is the offer, not the walk. The container, the manifest grammar, and the reader are [module bundles](bundles.md).":
        '两种根都可加载（编译根连同其组与打包期作用域台账，在世界收束时重定基；声明根作为该包的宿主行），先挂载者胜，捆绑包根的包名就是 `loaded.root`。wasm 宿主对已提交工件 `include_bytes!` 并经 `Pkg::from_bundle` 解析。至于 url 依赖的_遍历_（固定值、闭包检查、档案组上的对等门）请留在 `load_dir*` 车道——捆绑包挂载是 offer，不是遍历。容器、清单语法与读取器见[模块捆绑包](bundles.md)。',
    "**What to take from a url** is the engine's instantiation law, read from the embedding side: a compiled bundle serves host surfaces, concrete-class libs, **and** — by the generic-source riding law — the generic owners: a request the pack-time ledger lacks lowers the ridden source in the consumer's world and compiles the monomorphized body under the declaring pkg's spec, at the link, nothing persisted (`Vec<MyTodo>`, `decodeJson<T>` — [module bundles](bundles.md) — the std-CDN section). A bundle that owns an open generic surface but carries no riding source refuses at the mount (`pouch` owns an open generic surface but its bundle carries no riding source — re-pack the directory), and a bundle-mounted json names its pack-time dev closure in its ledger, so the consumer's closure must contain those names.":
        '**从 url 能拿到什么**是从嵌入一侧读到的引擎实例化法则：编译捆绑包服务宿主表面、具体类库，**还有**——按泛型源码承载法则——泛型 owner：打包期台账没有的请求会在消费者的世界里降低被承载的源码，并以声明 pkg 的 spec 在链接处编译单态化代码体，不持久化任何东西（`Vec<MyTodo>`、`decodeJson<T>`——[模块捆绑包](bundles.md)——std-CDN 一节）。拥有开放泛型表面却不带承载源码的捆绑包在挂载时拒绝（`pouch` owns an open generic surface but its bundle carries no riding source — re-pack the directory），包装载的 json 还在其台账中命名其打包期开发闭包，因此消费者的闭包必须包含那些名字。',
    'pack a module directory into a deterministic `.rutbundle` — a **compiled** root for a lib pkg, a **decl** root for a host pkg (`type` routes; `format_version` is 10); returns the bytes ([module bundles](bundles.md))':
        '把模块目录打包成确定性的 `.rutbundle`——lib 包为**编译**根，宿主包为**声明**根（由 `type` 路由；`format_version` 是 10）；返回字节（[模块捆绑包](bundles.md)）',
    '`Logger("app")` constructs through the class\'s `[constructor]` member — the call form spells the construction.':
        '`Logger("app")` 经由类的 `[constructor]` 成员构造——调用形式拼写的就是构造。',
    'Both classes designate `new` — `HashMap()` / `HashSet()` spell the construction; `with_capacity` stays a named constructor.':
        '两个类都指定 `new`——`HashMap()` / `HashSet()` 拼写构造；`with_capacity` 仍是命名构造器。',
    '**Inlining** — single-callee calls and small bodies (a callee op-count budget). Cross-module inlining after linking is future work; binaries carry no cross-function inlined code.':
        '**内联**——单一被调用者的调用与小函数体（按被调用者的操作数预算）。链接后的跨模块内联是未来工作；二进制不携带跨函数内联代码。',
    "The `Surface` is the export record — functions, constants, types, interface declarations, inherent method surfaces, and the builtin names `core` publishes. It is how a _using_ module binds a _used_ module's members at compile time, and it is part of the wire format too — the surface section rides after `exports`, and decode validates every row against the tables above it.":
        '`Surface` 是导出记录——函数、常量、类型、接口声明、固有方法表面，以及 `core` 发布的内建名。_使用_模块在编译期据此绑定_被使用_模块的成员，它也是线上格式的一部分——表面节在 `exports` 之后承载，解码会对照上方的表验证每一行。',
    "**Instantiation ledger**: the owner-anchored identity of every instantiation the program compiled — type rows `(owner, decl, arguments, type id)` first, then fn identities (the kind, the substitution values, and the local fn id). Instantiation is owned by the declaring package, and link unifies rows sharing a key into ONE program-wide row; a consumer resolves its requests against a packaged binary's ledger. Type exports carry their generic parameter lists beside this (see the surface below).":
        '**实例化台账**：程序编译过的每个实例化的属主锚定身份——先是类型行 `(owner, decl, arguments, type id)`，再是函数身份（种类、替换值、局部 fn id）。实例化归声明包所有，链接把共享同一键的行统一为全程序唯一的一行；消费方针对打包二进制的台账解析自己的请求。类型导出在其旁携带自己的泛型参数表（见下方表面）。',
    "**Surface**: the exported surface rides after `exports` — the namespace head, funcs (with their async/host rows), consts, the carried type descriptors + scope blocks + type exports (generic exports carry their parameter names in order), interface declarations (`SurfaceIface` — every module publishes the interfaces it declares; a consumer links them into one merged table), the inherent method surfaces (the linkable-classes phase's one row per class with methods), and the native rows with their ambient bits. There are no impl-registration rows on the wire — satisfaction is structural, so nothing registers. Names are ids into the name table above; decode rejects any id its tables cannot resolve.":
        '**表面**：导出的表面在 `exports` 之后承载——命名空间头、函数（带其 async/host 行）、常量、随行的类型描述符 + 作用域块 + 类型导出（泛型导出按顺序携带自己的参数名）、接口声明（`SurfaceIface`——每个模块都发布自己声明的接口；消费者把它们链接成一张合并表）、固有方法表面（可链接类阶段每个有方法的类一行），以及带环境位的 native 行。线上没有 impl 注册行——满足是结构化的，没有东西需要注册。名字是上方名字表的 id；解码拒绝它的表无法解析的任何 id。',
    "**Versioning policy**: the version `u32` must equal the toolchain's exactly — there is no migration or best-effort decode. A byte that changes observable behavior bumps the version; artifacts from older compilers are refused with the standard version error.":
        '**版本策略**：版本 `u32` 必须与工具链的完全一致——没有迁移，也没有尽力解码。改变可观察行为的字节会推进版本；旧编译器的工件以标准的版本错误拒绝。',
    'Allocation strategy':
        '分配策略',
    '**No compaction.** Non-moving keeps host borrows into `Vec` buffers sound and freelists trivial.':
        '**没有压缩。** 不移动让指向 `Vec` 缓冲的宿主借用保持健全，空闲列表保持平凡。',
    '`Vec` growth charges the budget before the write; shrinking is never refunded (the accounting overcounts rather than undercounts).':
        '`Vec` 增长在写入之前计入预算；收缩绝不退款（记账宁可多计，不少计）。',
    'Launched futures are **unstructured**: aborting a frame does not abort frames it awaits. Structured scopes — `scope { .. }` cancelling children on exit — are the specified remedy.':
        '已启动的 future 是**非结构化的**：abort 一个帧不会 abort 它所 await 的帧。结构化作用域——`scope { .. }` 在退出时取消子帧——是已规定的补救。',
    '`main.rs` packs the same directory with `rut_native::pack_dir_opts_with` — a **compiled** bundle: the plugin rides as a `.rutc` binary, its host pkg `server` as a source group — writes `plugin.rutbundle` to temp, and loads it back through the identical `Plugin::load`. The transcript equality print is the proof. The CLI drives the same loader for any self-contained module:':
        '`main.rs` 用 `rut_native::pack_dir_opts_with` 打包同一目录——一个**编译**捆绑包：插件以 `.rutc` 二进制承载，其宿主包 `server` 作为源码组——把 `plugin.rutbundle` 写到临时位置，再经同一个 `Plugin::load` 载回。转录相等的打印就是证明。CLI 用同一个加载器驱动任何自包含的模块：',
    '`new` is not special syntax — just the conventional primary-constructor name (`from`, `parse`, `open`, `default` are its siblings); it is an ordinary identifier. Try-construction returns the nullable: a class method `fn parse(s: str) -> ?Version` answers `nil` on failure. A class may designate its primary constructor with `[constructor]` — then the call form spells the construction too: std\'s `Logger("t")` constructs through the `[constructor]` member (the `[constructor]` section below).':
        '`new` 不是特殊语法 —— 只是约定俗成的主构造方法名（`from`、`parse`、`open`、`default` 是它的同辈）；它是一个普通标识符。尝试式构造返回可空类型：类方法 `fn parse(s: str) -> ?Version` 在失败时应答 `nil`。类可以用 `[constructor]` 指定它的主构造器——此后调用形式也能拼写构造：std 的 `Logger("t")` 经由 `[constructor]` 成员构造（见下文的 `[constructor]` 一节）。',
    'The patterns are: enum members (`Light.Red`), literals (integers, floats, `bool`, `str`), comma-separated alternatives, and `else`. There are no ranges, no destructuring, and no guards — an `if` inside the arm body does that job.':
        '模式有：枚举成员（`Light.Red`）、字面量（整数、浮点数、`bool`、`str`）、逗号分隔的多选一，以及 `else`。没有范围，没有解构，也没有守卫——分支体内的 `if` 承担这份工作。',
    'A _generic class_ in a parameter position does not unify — `fn sum(xs: Vec<i32>)` is fine, but a function generic over `T` taking `Vec<T>` is not yet the shape to reach for. Concrete instantiations cover most code.':
        '参数位置上的_泛型类_不做统一——`fn sum(xs: Vec<i32>)` 没问题，但对 `T` 泛型又接收 `Vec<T>` 的函数还不是值得选用的形状。具体实例化覆盖大多数代码。',
    "Keys come from a fixed set — integers, `bool`, `str`, `bytes` (no floats: they have no stable equality contract) — hashed by the host; a user-defined key escapes by encoding canonically to `bytes`. A hit returns the stored cell, not a copy. There is no iteration surface: maps and sets answer questions, they don't walk. Construction spells the call form — `HashMap()` / `HashSet()` are the designated `new`; `with_capacity(n)` stays named.":
        '键来自一个固定集合——整数、`bool`、`str`、`bytes`（没有浮点数：它们没有稳定的相等契约）——由宿主做哈希；用户自定义的键以规范编码成 `bytes` 的方式破例。命中返回的是存储的单元，不是副本。没有迭代表面：map 和 set 回答问题，不做遍历。构造以调用形式拼写——`HashMap()` / `HashSet()` 是被指定的 `new`；`with_capacity(n)` 仍是命名构造器。',
    '`StringBuilder()` is the designated `new`; `with_cap(n)` pre-sizes and stays named.':
        '`StringBuilder()` 是被指定的 `new`；`with_cap(n)` 预先定容，仍是命名构造器。',
    '**`Logger("hello")`** constructs the logger through the class\'s `[constructor]` member — the call form spells the construction ([classes](../reference/classes.md)).':
        '**`Logger("hello")`** 经由类的 `[constructor]` 成员构造 logger——调用形式拼写的就是构造（[类与构造器](../reference/classes.md)）。',
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
