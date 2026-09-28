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
| downcast | 向下转型 | prose only; `opaque.downcast<T>` stays code |
| fuel | 燃料 | out of fuel → 燃料耗尽 |
| budget | 预算 | heap budget → 堆预算 |
| heap | 堆 | the Rc heap → Rc 堆；the VM heap → VM 堆 |
| host | 宿主 | host application → 宿主应用；host fn → 宿主函数 |
| embedder | 嵌入方 | embedding → 嵌入 |
| boundary | 边界 | host boundary → 宿主边界；value boundary → 值边界 |
| trait | trait | 保留不译（Rust 中文社区惯例；勿译作"特征/特质"） |
| impl block | impl 块 | `impl` is a keyword and stays code |
| dispatch | 分派 | dynamic dispatch → 动态分派；method dispatch → 方法分派 |
| task | 任务 | |
| future | future | 保留不译（概念/类型名；勿译作"未来"） |
| cancellation | 取消 | cancellation flag → 取消标志；checkpoint → 检查点 |
| channel | 通道 | workers and channels → worker 与通道 |
| worker | worker | 保留不译（rut 的独立 VM 隔离单元，非泛指线程） |
| module | 模块 | |
| package | 包 | distinct from bundle |
| manifest | 清单 | the manifest → 清单（`rut.toml` stays code） |
| bundle | 捆绑包 | module bundle → 模块捆绑包（`mod.rutbundle` stays code） |
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
| destructor | 析构器 | drop → 保留 drop（drop path → drop 路径） |
| weak reference | 弱引用 | |
| cycle | 循环引用 | the cycle collector → 循环收集器 |
| string view | 字符串视图 | slicing → 切片 |
| standard library | 标准库 | |
| playground | Playground | 专有名词，不译 |
| compile | 编译 | the compiler → 编译器；frontend → 前端 |
| symbolication | 符号化 | |
| embed loop | 嵌入循环 | |

First mention of a glossary term in a chapter may carry the English in
parentheses — 中文（English） — when the mapping is not obvious from
context; after that, use the zh-CN rendering alone.

## Hard rules

1. **Code never translates.** Inline code spans, fenced code blocks,
   identifiers, type names, field/method names, keywords, numeric
   literals, and URLs stay byte-identical: `rut`, `opaque`,
   `opaque.downcast<T>`, `pub fn main`, `rut.toml`, `.d.rut`,
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
