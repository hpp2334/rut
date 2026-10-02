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
    # -- the tasks cutover: the book speaks of launched futures, not tasks ----
    'Launched futures':
        '已启动的 future',

    'Async: futures, workers, and channels':
        '异步：future、worker 与通道',

    'Next: [async: futures, workers, and channels](async.md).':
        '下一章：[异步：future、worker 与通道](async.md)。',

    "rut's concurrency is **pull-based**. An `async fn` compiles into a _future_ — a cold value that runs nothing until something drives it. There are no promises that start on creation, no microtask queue, no implicit scheduling: the host owns time, and code progresses only when a driving loop pumps it. The model is described in [the async model](../core-concepts/async-model.md), with the full surface in [async and await](../reference/async.md) and [launched futures](../reference/tasks.md).":
        'rut 的并发是**拉取式**的。`async fn` 编译成一个 _future_——一个冷的值，在被某种东西驱动之前什么也不做。没有一创建就启动的 promise，没有微任务队列，没有隐式调度：时间由宿主掌握，代码只有在驱动循环泵动它时才前进。这个模型在[异步模型](../core-concepts/async-model.md)中描述，完整表面见[异步与 await](../reference/async.md)与[已启动的 future](../reference/tasks.md)。',

    "Racing futures (`await select { fut1 -> .., fut2 x -> .. }`) and joining a launched future's value (`await handle`) are spelled in the grammar but not in this build — the compiler gates them. Cancellation _is_ here: `handle.abort()` flags the frame, and the probe at its next checkpoint unwinds it deterministically, running cleanup in reverse declaration order. See [launched futures](../reference/tasks.md) for the roadmap.":
        '竞速 future（`await select { fut1 -> .., fut2 x -> .. }`）与接取已 launch 的 future 的值（`await handle`）在语法里写得出来，但在当前构建中不可用——编译器把它们拦下。取消_已经落地_：`handle.abort()` 给帧打上标记，下一个检查点处的探针确定性地将它展开，按声明的逆序运行清理。路线图见[已启动的 future](../reference/tasks.md)。',

    'The async HTTP client lives with the concurrency chapter — builder construction, `send(cx)` resolving at headers, body drains and byte streams — in [async: futures, workers, and channels](async.md).':
        '异步 HTTP 客户端与并发那一章同住——构建器的构造、在响应头处完成的 `send(cx)`、响应体排干与字节流——见[异步：future、worker 与通道](async.md)。',

    "**The first parameter is the context.** `async fn f(cx: RunContext, ..)` — the engine mints it at call sites and per drive, the way it mints `self`. It carries the frame edge: `checkpoint()` reads the resume state, `cancelled()` reads the frame's abort flag. The spelled `RunContext` name is core's, imported like any package name: `use core::{ RunContext }` — the engine's weave itself never needs the import, only source that names the trait does.":
        '**第一个参数是上下文。**`async fn f(cx: RunContext, ..)`——引擎在调用点和每次驱动时铸造它，如同铸造 `self`。它携带帧边：`checkpoint()` 读取恢复状态，`cancelled()` 读取帧的取消标志。拼写的 `RunContext` 名字属于 core，像任何包名一样导入：`use core::{ RunContext }`——引擎的织造本身从不需要导入，需要它的是点名该 trait 的源码。',

    'The language today keeps the vocabulary deliberately small: launch, abort, and await. Racing (`await select { .. }`) and joining a launched future (`await handle`) parse but are compile-gated — the diagnostics name them as future work, and structured scopes (a block that cancels its children on exit) are the same story. What exists now is already enough to structure real programs — the launch/abort receipt gives you explicit ownership of background work, and drop-based cancellation gives it a clean off switch — but nothing in the model silently cancels siblings on your behalf. See the reference on [async and await](../reference/async.md), [launched futures](../reference/tasks.md), and the [host futures bridge](../reference/host-futures.md); the [GitHub viewer CLI](../examples/06-github-viewer-cli.md) example shows a full program living inside this loop.':
        '这门语言今天刻意保持很小的词汇表：launch、abort 和 await。竞速（`await select { .. }`）与 join 已启动的 future（`await handle`）能解析但被编译期闸住——诊断把它们标为未来工作，结构化作用域（一个在退出时取消其子帧的块）也是同样的情况。现有机制已足以组织真实的程序——launch/abort 回执让你对后台工作拥有显式所有权，基于 drop 的取消给了它一个干净的开关——但模型中没有任何东西会替你悄悄取消兄弟 future。参见关于[异步与 await](../reference/async.md)、[已启动的 future](../reference/tasks.md)与[宿主 future 桥](../reference/host-futures.md)的参考章节；[GitHub 查看器 CLI](../examples/06-github-viewer-cli.md) 示例展示了一个完整程序如何活在这个循环里。',

    "The receiver **is** the frame — the machine's fields are its state — and the context is the only handle a resumption needs. The frozen context protocol reads as data: `checkpoint` answers this frame's resume state, `cancelled` answers the frame's abort flag. Cancellation is a value the frame inspects, not an exception it catches.":
        '接收者**就是**帧——机器的字段就是它的状态——而上下文是一次恢复所需的唯一句柄。冻结的上下文协议按数据来读：`checkpoint` 回答本帧的恢复状态，`cancelled` 回答帧的中止标志。取消是帧检查的一个值，不是它捕获的异常。',

    "The returned `CustomLaunched` receipt is deliberately **not** a future — it cannot be awaited, and it is not re-launchable (that is a type error, never a runtime check). Its one real member is the cancel edge, which flags the frame and lets the loop's re-drive run the probe:":
        '返回的 `CustomLaunched` 回执刻意**不是** future——它不能被 await，也不能再次启动（那是类型错误，绝不是运行时检查）。它唯一真正的成员是取消边：标记帧，让循环的再驱动去跑那个探测：',

    '`rgh.rut` — the entry point is one `boot` fn: the host crosses argv in as a single `\\n`\\-joined string (no arg lists in the crossing set), and `boot` launches the brain with the standard launcher ([launched futures](../reference/tasks.md)):':
        '`rgh.rut` ——入口是一个 `boot` fn：宿主把 argv 以单个 `\\n`\\-连接的字符串跨越进来（可跨越集合里没有参数列表），`boot` 用标准启动器启动大脑（[已启动的 future](../reference/tasks.md)）：',

    'Cancellation drops locals at the suspension point through the same machinery ([launched futures](tasks.md)) — no special case.':
        '取消在挂起点透过同一机制丢弃局部变量（[已启动的 future](tasks.md)）—— 没有特例。',

    '`launch_future(launch_future(f))` is a **type error**: the receipt is not a `Future` ([launched futures](tasks.md)).':
        '`launch_future(launch_future(f))` 是一个**类型错误**：回执不是 `Future`（[已启动的 future](tasks.md)）。',

    'See [launched futures](tasks.md) for receipts, cancellation, and the join/select tier; [the host futures bridge](host-futures.md) for backing a host async fn with Rust; [workers and channels](workers-and-channels.md) for isolate parallelism. A worked user-defined future lives in [the custom-async example](../examples/04-custom-async.md).':
        '回执、取消与 join/select 层见[已启动的 future](tasks.md)；用 Rust 支撑宿主 async fn 见[宿主 future 桥](host-futures.md)；隔离体并行见 [worker 与通道](workers-and-channels.md)。一个完整可运行的用户自定义 future 在[自定义异步示例](../examples/04-custom-async.md)里。',

    "A launched future's receipt — `LaunchedFutureHandle<T>` — is its own type, and it is the entire management surface. Everything in this chapter is spelled in that vocabulary ([async and await](async.md)).":
        '已启动 future 的回执——`LaunchedFutureHandle<T>`——是它自己的类型，也是管理表面的全部。本章的一切都用这套词汇书写（[异步与 await](async.md)）。',

    'v1 launched futures are **unstructured**: aborting a frame does not abort frames it awaits. Structured scopes — `scope { .. }` cancelling children on exit — are the specified remedy.':
        'v1 已启动的 future 是**非结构化的**：abort 一个帧不会 abort 它所 await 的帧。结构化作用域——`scope { .. }` 在退出时取消子帧——是已规定的补救。',

    'The ready ring is **round-robin**: each drive runs a frame to its next park or completion, so one greedy future cannot starve the queue. Priorities are not in the model.':
        '就绪环是**轮转（round-robin）**的：每次驱动把一个帧运行到它的下一次停放或完成，因此一个贪婪的 future 无法饿死队列。优先级不在模型之中。',

    'drain the async ready queue once; returns frames run':
        '排空一次异步就绪队列；返回运行的帧数',

    'unfinished async work':
        '未完成的异步工作',

    '`async_engine` declares the engine rows (`__launch`, `__abort`, `__sleep`, `__sleep_yield`); `async_host` restores the typed surface: `launch_future(f: Future<T>) -> LaunchedFutureHandle<T>`, `LaunchedFutureHandle.abort() -> bool`, `sleep(ms: u32) -> Future<nil>`. Each embedder mounts the pair **and** installs `rut_std::async_host::pkg()`; a session that mounts neither has no launcher ([launched futures](tasks.md), [host futures](host-futures.md)).':
        '`async_engine` 声明引擎行（`__launch`、`__abort`、`__sleep`、`__sleep_yield`）；`async_host` 还原类型化面：`launch_future(f: Future<T>) -> LaunchedFutureHandle<T>`、`LaunchedFutureHandle.abort() -> bool`、`sleep(ms: u32) -> Future<nil>`。每个嵌入方都挂载这对**并**安装 `rut_std::async_host::pkg()`；两者皆未挂载的会话没有启动器（[已启动的 future](tasks.md)、[宿主 future](host-futures.md)）。',

    "`async fn` compiles to a state machine: each `await` is a checkpoint state in the hidden frame, resume dispatch is the existing jump-table op, locals become frame fields, and suspension is a plain return. The op set grows zero rows for this — the driven half is an ordinary trait-vtable call through the future's `yield` row. The full protocol lives in [Async and await](async.md) and [launched futures](tasks.md).":
        '`async fn` 编译成一个状态机：每个 `await` 是隐藏帧里的一个检查点状态，恢复分派是现有的跳转表操作，局部变量成为帧字段，挂起是一次普通的返回。为此操作集零新增——被驱动的那一半是一次普通的 trait-vtable 调用，走 future 的 `yield` 行。完整协议见[异步与 await](async.md)与[已启动的 future](tasks.md)。',

    'Every async entry runs as a root frame, parked when `await` returns pending. There is no microtask queue and no job executor — two queues and a virtual clock:':
        '每个异步入口都作为一个根帧运行，在 `await` 返回 pending 时停放。没有微任务队列，也没有作业执行器——只有两条队列和一个虚拟时钟：',

    'The engine never owns a wall clock: sleeps are virtual-clock deadlines, and tests virtualize time by advancing the clock — full determinism ([Async and await](async.md), [Launched futures](tasks.md)).':
        '引擎从不拥有墙上时钟：sleep 是虚拟时钟期限，测试通过推进时钟来虚拟化时间——完全确定性（[异步与 await](async.md)、[已启动的 future](tasks.md)）。',

    'Execution: `main` is called with no arguments; then the async driving loop runs — drain the ready queue, advance the virtual clock to the next timer deadline, repeat until no frames and no pending work remain. The loop is capped, so a program that never idles fails loudly instead of hanging.':
        '执行：`main` 无实参调用；然后异步驱动循环运行——排空就绪队列、把虚拟时钟推进到下一个定时器期限，如此往复，直到没有帧也没有待处理的工作残留。循环设有上限，因此永不空闲的程序会响亮失败而不是挂死。',
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
