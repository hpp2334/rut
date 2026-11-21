/* §MemberLaw — the one resolver hover, definition and completion share. */
import { html } from "./vdom.js";
import { CARD, CODE, PRE, GROUP_H, Section, Law } from "./ui.js";

export const MemberLaw = () => html`
  <${Section} id="member" num="03" title="The member law"
      kicker="Hover, go-to-definition and completion all ask the same question — where does this member declare? They share one resolver.">
    <div className=${CARD + " mb-4 p-5"}>
      <div className="grid items-center gap-4 md:grid-cols-[1fr_auto_1fr]">
        <div>
          <div className=${GROUP_H + " mt-0"}>own surface</div>
          <div className=${CODE}><pre className=${PRE}>${"struct Counter {\n  n: i32;      ← fields\n}"}</pre></div>
        </div>
        <div className="text-center text-2xl font-extrabold text-brand-600">∪</div>
        <div>
          <div className=${GROUP_H + " mt-0"}>inherent impl methods</div>
          <div className=${CODE}><pre className=${PRE}>${"impl Counter {\n  fn bump(self) …\n  fn reset(self) …\n}"}</pre></div>
        </div>
      </div>
      <p className="mb-0 mt-4 text-sm text-slate-500">Members of type <code className="rounded bg-slate-200/70 px-1.5 py-0.5 text-[.85em]">T</code> = T's own surface ∪
      inherent <code className="rounded bg-slate-200/70 px-1.5 py-0.5 text-[.85em]">impl T { .. }</code> methods. That is the whole list
      <code className="rounded bg-slate-200/70 px-1.5 py-0.5 text-[.85em]">c.⏐</code> offers.</p>
    </div>
    <div className="grid gap-3.5 md:grid-cols-2">
      <div className=${CARD + " p-5"}><div className="mb-1.5 font-bold">On an interface I</div>
        <p className="m-0 text-sm leading-relaxed text-slate-500">The members are <b>exactly</b> I's declared signatures —
        satisfaction is structural, so no impl lookup joins the two. A <i>foreign</i> interface's members
        resolve only when your document names it in a <code className="rounded bg-slate-200/70 px-1.5 py-0.5 text-[.85em]">use</code> (the use-both gate).</p></div>
      <div className=${CARD + " p-5"}><div className="mb-1.5 font-bold">The receiver resolves inside</div>
        <p className="m-0 text-sm leading-relaxed text-slate-500"><code className="rounded bg-slate-200/70 px-1.5 py-0.5 text-[.85em]">self</code> → the enclosing type · literals →
        primitives · a name → the locals pass (bindings), then module lets · a call → its written return.
        Callers hand the <b>node</b> under the cursor; they never pre-compute facts.</p></div>
    </div>
    <${Law}>The answer is a verdict: <code>Found(target)</code>, a <b>final miss</b> (receiver's type known),
    or <code>UnknownReceiver</code> (the caller may fall through to the bare-name search). A heuristic
    miss is empty — never wrong text.<//>
  <//>`;
