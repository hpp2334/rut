# zh-CN 术语表（GLOSSARY）— the binding zh-CN terminology contract

Every translator — human or agent — translating `docs/po/zh_CN.po`
**must follow this glossary**. English (`docs/src/` + `docs/book.toml`)
is the single source of truth: prose changes happen there first, and
the matching `po/zh_CN.po` entries are updated in the same change.
When a new domain term appears in the source, extend this glossary in
the same change that introduces its translation — do not improvise a
private rendering.

Until an entry is translated, the book falls back to the English
source text by design; a partially translated book is a valid book.

## Canonical renderings

| English | zh-CN | Notes |
| --- | --- | --- |
| value | 值 | "everything is a value" → 一切皆值 |
| cell | 单元 | a heap cell → 堆单元；refcounted cell → 引用计数单元 |
| handle | 句柄 | the handle copies, not the cell → 复制的是句柄，不是单元 |
| sharing | 共享 | "sharing is the law/default" → 共享是默认法则 |
| erasure box | 擦除盒 | `opaque` stays code: 擦除盒（`opaque`） |
| downcast | 向下转型 | prose only; `opaque::downcast<T>` stays code |
| fuel | 燃料 | out of fuel → 燃料耗尽 |
| budget | 预算 | heap budget → 堆预算 |
| heap | 堆 | the Rc heap → Rc 堆；the VM heap → VM 堆 |
| host | 宿主 | host application → 宿主应用；host fn → 宿主函数 |
| embedder | 嵌入方 | embedding → 嵌入 |
| boundary | 边界 | host boundary → 宿主边界；value boundary → 值边界 |
| trait | trait | 保留不译（Rust 中文社区惯例；勿译作"特征/特质"） |
| impl block | impl 块 | `impl` is a keyword and stays code |
| dispatch | 分派 | dynamic dispatch → 动态分派；method dispatch → 方法分派 |
| launched future | 已启动的 future | launch → 启动（launch_future stays code） |
| race | 竞速 | select2 / select_all stay code; race 的动词与名词均译作竞速；loser → 落败者 |
| future | future | 保留不译（概念/类型名；勿译作"未来"） |
| cancellation | 取消 | cancellation flag → 取消标志；checkpoint → 检查点 |
| channel | 通道 | workers and channels → worker 与通道 |
| worker | worker | 保留不译（rut 的独立 VM 隔离单元，非泛指线程） |
| module | 模块 | |
| package | 包 | distinct from bundle |
| manifest | 清单 | the manifest → 清单（`rut.jsonc` stays code） |
| JSONC | JSONC | 保留不译（格式名）；JSONC 切换 → the JSONC cutover；尾随逗号 → trailing commas；`//` 行注释 / `/* */` 块注释 stays code |
| bundle | 捆绑包 | module bundle → 模块捆绑包（`mod.rutbundle` stays code） |
| runnable unit | 可运行单元 | the manifest makes the directory THE runnable unit — `rut run` accepts dir-or-bundle only; a loose `.rut` file is not a program |
| declaration file | 声明文件 | `.d.rut` stays code |
| bytecode | 字节码 | typed bytecode → 类型化字节码 |
| VM | VM | 保留不译；"virtual machine" spelled out → 虚拟机 |
| trap | 陷阱 | runtime failure mode → 陷阱（trap）；首次出现标注英文。`Trap`（kind 名）与代码中的 trap 保留原文 |
| diagnostics | 诊断 | as prose often 诊断信息 |
| reflection | 反射 | never used for "reified" — see below |
| reified type | 具体化类型 | 首次出现标注英文（reified type）；与"反射"严格区分 |
| borrow | 借用 | |
| nullable | 可空 | nullable box → 可空盒；`?T` stays code |
| closure | 闭包 | |
| generic | 泛型 | |
| struct | 结构体 | `struct` keyword in code stays |
| enum | 枚举 | `enum` keyword in code stays |
| class | 类 | `class` keyword in code stays |
| newtype | 新类型 | the positional one-field wrapper decl（`class Name(Wrapped);` stays code）；首次出现标注英文 newtype |
| wrapper | 包装器 | the wrapper family → 包装器家族；wrapped type → 被包装的类型 |
| satisfaction | 满足 | structural satisfaction → 结构化满足（interfaces 语境；能力"够得着/够不着"以满足为准） |
| destructor | 析构器 | drop → 保留 drop（drop path → drop 路径） |
| weak reference | 弱引用 | |
| cycle | 循环引用 | the cycle collector → 循环收集器 |
| string view | 字符串视图 | slicing → 切片 |
| standard library | 标准库 | |
| playground | Playground | 专有名词，不译 |
| compile | 编译 | the compiler → 编译器；frontend → 前端 |
| symbolication | 符号化 | |
| embed loop | 嵌入循环 | |
| compiled group | 编译组 | a bundle dep whose `.rutc` rides（对照 source group） |
| source group | 源码组 | a host pkg (or a declared dep whose unit did not link) riding its declaration file set |
| riding (generic-source, RETIRED) | 承载（泛型源码承载，已废止） | the v22 delivery law repealed it — historical mentions only；the riding law → 承载法则（仅历史语境）；rides beside the binary → 在二进制旁承载 |
| delivery law (v22) | 交付法则（v22） | generics ride compiled: 泛型走编译交付；a published bundle never carries source → 已发布的捆绑包绝不携带源码；instantiation is owner-anchored → 实例化按拥有者锚定 |
| template (wire) | 模板 | generic bodies ride the binary as templates → 泛型代码体作为模板承载在二进制内；monomorphize from the template → 从模板单态化；template bodies → 模板代码体 |
| scope ledger | 作用域台账 | `rut.scopes` stays code；打包期作用域 → 打包期作用域 |
| rebase | 重定基 | packed ids rebase onto the loader's numbering |
| symbol stripping | 符号剥离 | the `--strip` lane — compile-time name/position stripping |
| sidecar | 旁车 | the `.rutsym` private symbol table rides beside the bundle；`.rutsym` stays code（与 compiler.md 的"旁车 map 文件"同一词） |
| symbol table | 符号表 | the private restore half; not a secret |
| mangled name | 改名后的名称 | the strip's rename — `%N` stays code；动词 mangle → 改名 |
| keep-set | 保留集 | names that never rename（host ABI、exports、pkg specs） |
| temporary-run lane | 临时运行车道 | the CLI's role — quick experiments (run/fmt/dump/pack)，不是交付路径；两条消费车道都叫车道 |
| git dependency | git 依赖 | the Cargo dependency form that ships the engine crates |
| pin | 固定值 | the sha256 pin → sha256 固定值；动词 pin（把依赖固定到某哈希）→ 固定；the pin's source is `rut.lock` — 固定值以 `rut.lock` 为源（the pin = lock law → 固定值即锁法则） |
| rut.lock | `rut.lock` | stays code（同 `rut.jsonc`）；the pin ledger → 固定值清单；one per walk root → 每个遍历根一份；committed → 随清单一起提交；byte-deterministic → 字节确定 |
| rut install | `rut install` | stays code；the lock-writing lane → 写入 `rut.lock` 的车道；`--locked` stays code（CI 校验车道，绝不写入） |
| walk root | 遍历根 | the lock pins the whole closure at the walk root → 锁在遍历根固定整个闭包 |
| fetch | 拉取 | the `DepRemote` contract → `DepRemote` 契约（代码不译）；`rut fetch` stays code（re-warm lane → 再预热车道） |
| bracket marker | 括号标记 | `[disposal]` / `[iterable]` / `[constructor]` stay code；标记词不译——方括号即指定 |
| marker | 标记 | the closed marker set → 封闭标记集；designation by bracket, not name → 以方括号指定，而非以名字指定 |
| designated surface | 指定表面 | the three designated surfaces：`[disposal]`（引擎事件）、`[iterable]`（for-of 脱糖）、`[constructor]`（用户调用的 `Type(..)`）——stay code；the designation routes the call → 指定路由调用；designate → 指定 |
| designated slot | 指定槽位 | for-of reads this slot → for-of 读取该指定槽位 |
| mint (verb) | 铸造 | the engine mints the frame → 引擎铸造帧；engine-minted → 引擎铸造 |
| async block | async 块 | `async { .. }` stays code；the async primitive → async 原语 |
| inject | 注入 | the weave injects the resume context → 编织注入恢复上下文；injected, never spelled → 注入，绝不拼写 |
| remote (policy) | 远程（策略） | the remote policy → 远程策略；`DepRemote`/`HttpRemote`/`Loader`/`Loaded` stay code |
| cache-first | 缓存优先 | a hit never touches the network → 命中则绝不触碰网络 |
| offline remote | 离线远程 | `HttpRemote::offline` — cache-only → 只用缓存；prime the cache → 预热缓存 |
| poisoned entry | 被污染的缓存条目 | the eviction law → 驱逐法则 |
| mount door | 挂载门 | the loader's mount point where the pin is law — 每次加载都校验 |
| rode-along group | 随行组 | a url dep's groups riding a consumer's pack |
| nameability | 可命名性 | the declaration-scoped use law → 可命名性按声明划定（nameability is declaration-scoped）：一个包只能 `use` 自己清单点名的头（`deps` ∪ 根上的 `dev-deps` ∪ `core`）；transitive mount ≠ nameable → 传递挂载不等于可命名；D5 stays code |
| content identity | 内容标识 | a bundle group's dirname: `name@<16 hex>` — sha256 over the group's own rode source set (`rut.jsonc` + `mod.rut` + `mod.d.rut` + `rut.mods`, fixed order, absent skipped)，`name@<fp16>` stays code；同一源码 ⇒ 同一标识，同一标识只挂载一次（精确去重） |
| coexistence | 共存 | 同名两个内容标识并肩挂载；多标识之下裸 `use` 头是响亮的歧义——用自己的 `deps` 行固定其一，或经作用域台账绑定 |

First mention of a glossary term in a chapter may carry the English in
parentheses — 中文（English） — when the mapping is not obvious from
context; after that, use the zh-CN rendering alone.

## Hard rules

1. **Code never translates.** Inline code spans, fenced code blocks,
   identifiers, type names, field/method names, keywords, numeric
   literals, and URLs stay byte-identical: `rut`, `opaque`,
   `opaque::downcast<T>`, `entry fn main`, `rut.jsonc`, `.d.rut`,
   `mod.rutbundle`, `rc`, `spawn`, CLI verbs (`rut run`, `rut build`,
   `rut check`, `rut fmt`, `rut doc`, …), package names (`core`,
   `ink`, `pouch`, …).
2. **Product and tool names stay unchanged**: mdbook, mdbook-i18n-helpers,
   Cloudflare Pages, Cloudflare Workers, highlight.js, GitHub, Rust,
   Cargo, wasm/WebAssembly.
3. **zh-CN punctuation in prose**: use ，。：；？！ and full-width
   parentheses （）, quotes "" '' — never half-width `, . : ; ? ! ( )`
   in Chinese sentences. Punctuation INSIDE code spans, fenced blocks,
   link targets, and identifiers is exempt (rule 1). The delimiter of a
   full-width parenthetical touching a code span is still full-width:
   擦除盒（`opaque`）.
4. **Markdown and hljs semantics are load-bearing.** Do not add/remove
   or reorder markdown structure: heading levels, list markers, table
   pipes and column counts, link reference definitions, emphasis
   markers, and fenced-block info strings (` ```rut `) must survive
   translation exactly. Highlight spans are baked client-side from the
   fence language — never hand-insert `<span>` classes or HTML into
   translated prose. Link TEXT may be translated; link TARGETS may not.
5. **Fenced code blocks are translated only in their comments** (and
   only when marked translatable); string literals and code text stay
   as-is unless the surrounding prose explicitly presents them as
   illustrative output. When in doubt, leave the block untouched.
6. **Chapter/section titles follow the glossary** and stay short; the
   sidebar, search index, and anchors derive from them. Do not add the
   English in parentheses to titles.
| seeded fetcher | 预置取回器 | RETIRED — the seed shape is gone; the remote policy is named explicitly now（远程策略） |
| builder | 构建器 | `HostPkg` builder → `HostPkg` 构建器（`HostPkg` stays code）；`Vm::builder()` stays code |
| installer lane | 安装者车道 | 对照 the raw lane → 原生车道；宿主函数体经 `.host_pkg(..)` 上链安装 |
| rows snapshot | 行快照 | RETIRED as `mount snapshot` — the snapshot now reads off the offered pkgs（`rut_driver::host_pkg_ctx(pkgs)` stays code） |
| first-mount-wins | 先挂载者胜 | |
| inert merge | 惰性合并 | inert extras → 惰性额外物 |
| decl root / compiled root | 声明根 / 编译根 | the v6/v5 pairing → 配对是完全的（the pairing is total） |
| walk | 遍历 | rut-native 的目录遍历（the walk）；the deps walk → deps 遍历；动词 walk → 遍历；对照 offer |
| offer | offer | 保留不译（`.pkg(..)` 的提供法则，名词）；动词 offer(s) → 提供；first-pkg-wins → 先提供者优先；the offer lane → offer 车道 |
| first-pkg-wins | 先提供者优先 | 链上的 first-mount-wins（先挂载者胜） |
| file module | 文件模块 | a `mod NAME;`-declared child: `NAME/mod.rut` beside the declaring file, or the single-file `NAME.rut`（`mod` 为代码） |
| directory form | 目录形式 | the `NAME/mod.rut` child spelling — probed first |
| single-file form | 单文件形式 | the `NAME.rut` child spelling beside the declaring file; mounts at the logical mod path `NAME` exactly as the directory would |
| both spellings | 两种拼法 | both forms present is the loud ambiguity — keep one |
| root module | 根模块 | the package's `mod.rut` beside its manifest |
| module tree | 模块树 | the mounted file-module set of a package |
| qualified position | 限定位置 | the `::`-spelled intra-package reference（`layout::Column` stays code）；dot → 点号（`.` 只拼值访问） |
| use path | use 路径 | the `::`-spelled import path（`use` 为代码） |
| intra-package import | 包内导入 | the own-package use lane — a use head naming the consumer's own package（its name → the root module，a `mod` edge → the walked child）；own package → 本包；own module edge → 本包模块边 |
| path separator | 路径分隔符 | `::` — THE path separator: every name-resolution path spells its segments with it（每条名称解析路径的段都由它拼写）；`.` is value access only |
| value access | 值访问 | fields, instance methods, tuple indices → 字段、实例方法、元组索引（`.` 的唯一职责） |
| mount | 挂载 | mounting is declared, not discovered → 挂载由声明决定，而非目录发现 |
| declarations are the graph | 声明即图 | the `mod` decls ARE the module set — never a directory listing |
| repealed | 已废除 | a retired manifest key refuses loudly → 被废除的键会大声拒绝并给出修复方法 |
| reader compat | 读取器兼容 | old published bundle envelopes keep loading — a loading law |
| module rows | 模块行 | the `rut.mods` additive envelope section（path-keyed source rows） |
| riding source | 承载源码 | generic-source riding：the source that serves consumer-spelled shapes |
| run chain | 运行链 | `RutRun::new()..compile()` 的组合 —— `Pkg` → `RutRun` → `Compiled` → `Vm` 一值贯通 |
| auto-ride / auto-offer | 自动承载 / 自动 offer | core 在 `.compile()` 中自动承载（riding → 承载）；除非已 offer 了名为 `core` 的包 |
| yield | 产出 | a walk's yield → 遍历的产出（`Loaded { pkgs, root }`） |
| world | 世界 | the walk's world → 遍历的世界；close the world → 收束世界；the consumer's world → 消费者的世界 |
| named record | 具名记录 | user struct/class rows; never cross → 具名记录永不跨越 |
| anonymous tuple | 匿名元组 | the `(T, ..)` spelling crosses field-by-field → 匿名元组逐字段跨越 |
| scaffold | 脚手架 | 动词 scaffold → 搭建脚手架；`rut new` scaffolds a directory → 为目录搭建脚手架 |
| clobber | 覆盖 | refuse to clobber → 拒绝覆盖（已有的文件）；clobber refusal → 拒绝覆盖 |
| basename | 基名 | the target's basename → 目标的基名 |
| derived name | 派生名 | the package name derived from the basename → 从基名派生的包名 |
| every-field-covered law | 字段全覆盖法则 | 字面量必须覆盖每个字段——在字面量里写出，或由该字段的声明初始化器（declared initializer）覆盖；省略无初始化器字段是编译错误（`missing field … (the every-field-covered law)` 诊断原文保留） |
| duplicate field | 重复字段 | `duplicate field … in the … literal` 诊断原文保留；无“后者胜”（last-wins） |
| unknown import | 未知导入 | use 行即拒绝点；`` `X` has no `Y` — unknown import `` 诊断原文保留 |
| tail | 尾表达式 | block tail → 块的尾表达式；紧贴块的 `}`、不带 `;` 的最后一个表达式，即块的值；尾位置（tail position）；无尾表达式 → 无尾/保持无尾；首现标注英文（tail） |
| `;`-decides law | “`;` 说了算”法则 | `;` 收尾的语句丢弃其值，紧贴 `}` 的表达式是块的尾表达式；`let` 始终要求 `;` |
| implicit return | 隐式返回 | fn 体/lambda 块体的尾表达式即隐式返回；definite-return law → 必然返回法则（`missing return: …` 诊断原文保留） |
| `if` atom | `if` 原子 | `if` 的表达式形式（`if` stays code）；parens required → 括号不可省；statement form → 语句形式；expression form → 表达式形式 |
| value block | 值块 | 产生值的块；value-producing arm → 产生值的分支；arm → 分支（`when`/`if` 的分支，`->` 之后的部分） |
