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
# DESIGN. This sweep (Weak is import-gated — `pub builtin`, not
# `prelude builtin`) changes FIFTEEN strings: fourteen are prose
# rewrites (the ambient/gated split moves `Weak`) or code blocks that
# gained the `use core::{ Weak };` line (code never translates — the
# msgstr IS the msgid), one is the new weak-refs gating paragraph.
FRESH = {
    # ---- the weak-import-gated sweep: `Weak` leaves the ambient prelude ----
    # 14 msgids changed (prose rewrote or a code block gained the use
    # line), 1 msgid is genuinely new (weak-refs.md's gating paragraph).
    # Code blocks: code never translates — the msgstr IS the msgid.
    # Terminology per docs/po/GLOSSARY.md: 导入把守 (import-gated),
    # 环境自带 (ambient), 弱引用 (weak reference).

    # tutorial/modules.md — the gated-names paragraph widens to `Weak`.
    "Builtin names — the primitives, `str`/`bytes` members, `panic`, `Vec`\\-free array grammar, `opaque` — are **ambient**: no `use` needed. The exceptions are core's import-gated names — the `Disposal`/`DisposalContext` pair and the weak reference `Weak` — they resolve only through `use core::{ .. }`, like any package name. Package names from your manifest are imported the same way; an unused name in a `use` is a lint, not an error.":
        "内置名字——基本类型、`str`/`bytes` 的成员、`panic`、不依赖 `Vec` 的数组语法、`opaque`——都是**环境自带**的：无需 `use`。例外是 core 的导入把守名字——`Disposal`/`DisposalContext` 对与弱引用 `Weak`——它们只能像任何包名一样，经 `use core::{ .. }` 解析。清单里的包名也以同样的方式导入；`use` 里未被使用的名字只是一个 lint，不是错误。",

    # core-concepts/memory.md — the Weak references section names the gate.
    "`Weak.new(v)` mints a `Weak<T>` over `v`'s cell that **never keeps anything alive**; `w.upgrade()` answers `?T` — the retained referent, or `nil` once it died. Construction is the class-method form, admission is checked (reference types only — `Weak<i32>` diagnoses), and `Weak.new(nil)` traps. The name is **import-gated** — spell `use core::{ Weak };`, or the bare `Weak` diagnoses `` `Weak` is not in scope — `use core::{ Weak }` ``.":
        "`Weak.new(v)` 在 `v` 的单元上构造一个 `Weak<T>`，它**永远不让任何东西保活**；`w.upgrade()` 回答 `?T`——被保留的引用对象，或它死掉之后的 `nil`。构造是类方法形式，准入受检查（仅限引用类型——`Weak<i32>` 会被诊断），`Weak.new(nil)` 触发陷阱。名字是**导入把守**的——拼写 `use core::{ Weak };`，否则裸写的 `Weak` 会诊断 `` `Weak` is not in scope — `use core::{ Weak }` ``。",

    # core-concepts/memory.md — the observer code block gained the use line.
    "```rut\nuse core::{ Weak };\nuse ink::{ Logger };\n\nclass Model {\n    name: str;\n}\n\nclass View {\n    model: ?Model;\n    observer: ?Weak<Model>;      // a back-pointer that closes no cycle\n}\n\nentry fn main() {\n    let log = Logger.new(\"rc\");\n    let mut m = Model { name: \"doc\" };\n    let v = View { model: nil, observer: Weak.new(m) };   // observe without owning\n    log.info(f\"holding {m.name}; the view holds only a weak edge\");\n    m = Model { name: \"next\" };   // the old cell's last strong reference dies here\n    log.info(f\"upgrade() answers nil: {v.observer.upgrade() == nil}\");\n}\n```":
        "```rut\nuse core::{ Weak };\nuse ink::{ Logger };\n\nclass Model {\n    name: str;\n}\n\nclass View {\n    model: ?Model;\n    observer: ?Weak<Model>;      // a back-pointer that closes no cycle\n}\n\nentry fn main() {\n    let log = Logger.new(\"rc\");\n    let mut m = Model { name: \"doc\" };\n    let v = View { model: nil, observer: Weak.new(m) };   // observe without owning\n    log.info(f\"holding {m.name}; the view holds only a weak edge\");\n    m = Model { name: \"next\" };   // the old cell's last strong reference dies here\n    log.info(f\"upgrade() answers nil: {v.observer.upgrade() == nil}\");\n}\n```",

    # reference/builtin-generic-types.md — the Weak<T> section names the gate.
    "`Weak<T>` is a builtin class whose box holds an _unretained_ word to a referent — a weak never keeps anything alive. The name is **import-gated**: spell `use core::{ Weak };` or the bare `Weak` diagnoses `` `Weak` is not in scope — `use core::{ Weak }` ``.":
        "`Weak<T>` 是一个内置类，其盒持有一个指向被引用对象的 _未保留_（unretained）字 —— 弱引用绝不会让任何东西存活。名字是**导入把守**的：拼写 `use core::{ Weak };`，否则裸写的 `Weak` 会诊断 `` `Weak` is not in scope — `use core::{ Weak }` ``。",

    # reference/builtin-generic-types.md — the construction block gained the use line.
    "```rut\nuse core::{ Weak };\nuse ink::{ Logger };\n\nstruct Tile { v: i32; }\n\nentry fn main() {\n    let log = Logger.new(\"t\");\n    let tile = Tile { v: 7 };\n    let w = Weak.new(tile);        // the class-method construction\n    let got: ?Tile = w.upgrade();  // the live referent, or nil once dead\n    log.info(f\"{got.v}\");\n}\n```":
        "```rut\nuse core::{ Weak };\nuse ink::{ Logger };\n\nstruct Tile { v: i32; }\n\nentry fn main() {\n    let log = Logger.new(\"t\");\n    let tile = Tile { v: 7 };\n    let w = Weak.new(tile);        // the class-method construction\n    let got: ?Tile = w.upgrade();  // the live referent, or nil once dead\n    log.info(f\"{got.v}\");\n}\n```",

    # reference/rc-dispose-identity.md — the weak block gained the use line.
    "```rut\nuse core::{ Weak };\nuse ink::{ Logger };\n\nstruct Tile { v: i32; }\n\nentry fn main() {\n    let log = Logger.new(\"t\");\n    let tile = Tile { v: 7 };\n    let w = Weak.new(tile);        // does NOT keep the cell alive\n    let got: ?Tile = w.upgrade();  // the live referent, or nil once dead\n    log.info(f\"{got.v}\");\n}\n```":
        "```rut\nuse core::{ Weak };\nuse ink::{ Logger };\n\nstruct Tile { v: i32; }\n\nentry fn main() {\n    let log = Logger.new(\"t\");\n    let tile = Tile { v: 7 };\n    let w = Weak.new(tile);        // does NOT keep the cell alive\n    let got: ?Tile = w.upgrade();  // the live referent, or nil once dead\n    log.info(f\"{got.v}\");\n}\n```",

    # reference/weak-refs.md — the NEW gating paragraph.
    "`Weak` is one of core's **import-gated** builtin names: spell `use core::{ Weak };` to bring the class in ([core and the swappable packages](stdlib.md)). A bare `Weak` does not compile — the diagnostic is `` `Weak` is not in scope — `use core::{ Weak }` ``.":
        "`Weak` 是 core **导入把守**的内置名字之一：拼写 `use core::{ Weak };` 把这个类引入（[core 与可换包](stdlib.md)）。裸写的 `Weak` 无法编译——诊断是 `` `Weak` is not in scope — `use core::{ Weak }` ``。",

    # reference/weak-refs.md — the first block gained the use line.
    "```rut\nuse core::{ Weak };\nuse ink::{ Logger };\n\nclass Node {\n    value: u32;\n    next: ?Node;          // strong — keeps the tail alive\n}\n\nimpl Node {\n    pub fn new(value: u32) -> Self { return Self { value: value, next: nil }; }\n}\n\nfn use_node(n: Node) {\n    let log = Logger.new(\"t\");\n    log.info(f\"node {n.value}\");\n}\n\nentry fn main() {\n    let n = Node.new(1);\n    let w = Weak.new(n);        // Weak<Node>; T infers from n\n    let b = w.upgrade();        // ?Node — a live handle\n    if (b != nil) {\n        use_node(b);\n    }\n}\n```":
        "```rut\nuse core::{ Weak };\nuse ink::{ Logger };\n\nclass Node {\n    value: u32;\n    next: ?Node;          // strong — keeps the tail alive\n}\n\nimpl Node {\n    pub fn new(value: u32) -> Self { return Self { value: value, next: nil }; }\n}\n\nfn use_node(n: Node) {\n    let log = Logger.new(\"t\");\n    log.info(f\"node {n.value}\");\n}\n\nentry fn main() {\n    let n = Node.new(1);\n    let w = Weak.new(n);        // Weak<Node>; T infers from n\n    let b = w.upgrade();        // ?Node — a live handle\n    if (b != nil) {\n        use_node(b);\n    }\n}\n```",

    # reference/weak-refs.md — the consuming-op block gained the use line.
    "```rut\nuse core::{ Weak };\nuse ink::{ Logger };\n\nstruct Payload { n: i32; }\n\nfn make() -> Payload { return Payload { n: 1 }; }\n\nentry fn main() {\n    let log = Logger.new(\"t\");\n    let v = make();\n    let w = Weak.new(v);  // watches the binding v — lives as long as v does\n    log.info(f\"{w.upgrade() != nil}\");\n}\n```":
        "```rut\nuse core::{ Weak };\nuse ink::{ Logger };\n\nstruct Payload { n: i32; }\n\nfn make() -> Payload { return Payload { n: 1 }; }\n\nentry fn main() {\n    let log = Logger.new(\"t\");\n    let v = make();\n    let w = Weak.new(v);  // watches the binding v — lives as long as v does\n    log.info(f\"{w.upgrade() != nil}\");\n}\n```",

    # reference/weak-refs.md — the disposal-ordering block merged Weak into its use line.
    "```rut\nuse core::{ Disposal, DisposalContext, Weak };\n\nclass Node  { child: ?Node; }\nclass Child { back: ?Weak<Node>; }   // the observer's weak back-pointer\n\nimpl Disposal for Child {\n    fn dispose(mut self, cx: DisposalContext) {\n        // the parent's death released self through the field walk,\n        // and the parent's weak list was nulled before any of that\n        // user code ran — the back-pointer reads nil from in here:\n        if (self.back.upgrade() == nil) { /* always taken here */ }\n    }\n}\n```":
        "```rut\nuse core::{ Disposal, DisposalContext, Weak };\n\nclass Node  { child: ?Node; }\nclass Child { back: ?Weak<Node>; }   // the observer's weak back-pointer\n\nimpl Disposal for Child {\n    fn dispose(mut self, cx: DisposalContext) {\n        // the parent's death released self through the field walk,\n        // and the parent's weak list was nulled before any of that\n        // user code ran — the back-pointer reads nil from in here:\n        if (self.back.upgrade() == nil) { /* always taken here */ }\n    }\n}\n```",

    # reference/host-fns.md — the ambient bullet loses `Weak`.
    "**`prelude builtin`** — the **ambient** engine surface: the name binds in every compilation unit, no `use` needed. The primitives and their `builtin impl` methods, `opaque`, `StackTrace`, and `panic`/`string_join`/`capture_stacktrace` are all ambient.":
        "**`prelude builtin`**——**环境自带**的引擎表面：名字在每个编译单元绑定，无需 `use`。基本类型及其 `builtin impl` 方法、`opaque`、`StackTrace`，以及 `panic`/`string_join`/`capture_stacktrace`，全是环境自带。",

    # reference/host-fns.md — the gated bullet gains `Weak<T>`.
    "**`pub builtin`** — the **import-gated** engine surface: the name resolves only through `use core::{ .. }`, the way a package's names do. Today's rows are every builtin trait — `Iterable`, `Future`, `RunContext` — the disposal pair, `Disposal` and `DisposalContext`, and the weak reference, `Weak<T>` ([traits](traits.md), [async and await](async.md), [the Rc heap](rc-heap.md), [weak references](weak-refs.md)). The engine's weave never consults the gate — it keys on the native-trait symbols — so a module with no imports still iterates, awaits, and launches; only spelling a name in source gates.":
        "**`pub builtin`**——**导入把守**的引擎表面：名字只能经 `use core::{ .. }` 解析，与包名的方式一样。如今的行是每个内置 trait——`Iterable`、`Future`、`RunContext`——单元死亡对 `Disposal` 与 `DisposalContext`，以及弱引用 `Weak<T>`（[trait](traits.md)、[异步与 await](async.md)、[Rc 堆](rc-heap.md)、[弱引用](weak-refs.md)）。引擎的织造从不查阅这道门——它以原生 trait 符号为键——因此一个没有任何导入的模块仍然可以迭代、await 与启动；把守只落在源码里拼写的名字上。",

    # reference/stdlib.md — the gated-names list gains `Weak<T>`.
    "The builtin fns, primitives, and containers are in scope in every compilation unit; no `use` is needed. A `use core::{ … };` statement stays legal but is redundant for them. The exceptions — the **import-gated** spellings, resolving only through `use core::{ .. }`: the const `NAN`, the `Disposal` pair (`Disposal`, `DisposalContext`), the weak reference `Weak<T>`, and every engine-woven trait (`Iterable`, `Future`, `RunContext`) — the engine's weave itself never needs the import, only source that spells the names (an `impl` block, a trait-typed signature, a `downcast<Future<..>>`). The two builtin spellings (ambient vs import-gated) are documented in [Host fns and declaration files](host-fns.md).":
        "内置函数、基本类型与容器在每个编译单元都在作用域内；无需 `use`。对它们而言 `use core::{ … };` 语句仍合法但多余。例外——**导入把守**的拼写，只能经 `use core::{ .. }` 解析：常量 `NAN`、`Disposal` 对（`Disposal`、`DisposalContext`）、弱引用 `Weak<T>`，以及每个引擎织入的 trait（`Iterable`、`Future`、`RunContext`）——引擎的织造本身从不需要导入，需要它的是拼写了名字的源码（一个 `impl` 块、一个 trait 类型的签名、一个 `downcast<Future<..>>`）。两种内置拼写（环境自带 vs 导入把守）记录在[宿主函数与声明文件](host-fns.md)。",

    # reference/stdlib.md — the construction note names the gate.
    "Construction keeps its builtin forms: `opaque(v)` seals, `Weak.new(v)` wraps (the class-method construction; the `Weak` name itself is import-gated — `use core::{ Weak }`) ([opaque](opaque.md), [weak references](weak-refs.md)).":
        "构造保留其内置形式：`opaque(v)` 封存，`Weak.new(v)` 包装（类方法构造；`Weak` 这个名字本身是导入把守的——`use core::{ Weak }`）（[opaque](opaque.md)、[弱引用](weak-refs.md)）。",

    # reference/stdlib.md — the builtin-classes table row gains the gate.
    "`Weak.new(v)` (traps on nil; reference types only), `upgrade() -> ?T` — `nil` once the referent died. **import-gated** — `use core::{ Weak }`":
        "`Weak.new(v)`（对 nil 触发陷阱；仅引用类型）、`upgrade() -> ?T`——所指者死亡后为 `nil`。**导入把守**——`use core::{ Weak }`",
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
