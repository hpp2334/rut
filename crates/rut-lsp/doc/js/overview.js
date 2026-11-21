/* §Overview — the hero + the layering diagram. */
import { html } from "./vdom.js";
import { CARD } from "./ui.js";
import { LAYERS } from "./data.js";

export const Overview = () => html`
  <section className="grid items-center gap-11 pb-8 pt-14 lg:grid-cols-[1.2fr_.9fr]" id="overview">
    <div>
      <span className="kind border border-brand-200 bg-brand-50 text-brand-600">interactive walkthrough</span>
      <h1 className="mt-3 text-[2.5rem] font-extrabold leading-[1.1] tracking-tight">How the rut<br/>language server works</h1>
      <p className="mt-4 max-w-[34em] text-[1.08rem] leading-relaxed text-slate-500">Every keystroke you
        type becomes hovers, completions, diagnostics and jumps through one small machine: an editable
        tree, a mounted world of files, and a set of questions you can ask them. This page walks that
        machine — and lets you poke at it.</p>
      <div className="mt-5 mb-6 flex flex-wrap gap-2">
        <span className="rounded-full border border-slate-200 bg-white px-3.5 py-1 text-xs font-semibold text-slate-500">✍️ for editor users</span>
        <span className="rounded-full border border-slate-200 bg-white px-3.5 py-1 text-xs font-semibold text-slate-500">🛠️ for contributors</span>
        <span className="rounded-full border border-slate-200 bg-white px-3.5 py-1 text-xs font-semibold text-slate-500">🔌 for host integrators</span>
      </div>
      <a href="#lab" className="inline-flex items-center gap-2 rounded-xl bg-brand-600 px-4.5 py-2.5 font-semibold text-white shadow-card transition hover:-translate-y-px hover:bg-brand-700">
        Try the completion lab
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round"><path d="M12 5v14m-6-6 6 6 6-6"/></svg>
      </a>
      <a href="#pipeline" className="ml-2.5 inline-flex items-center gap-2 rounded-xl border border-slate-200 bg-white px-4.5 py-2.5 font-semibold shadow-card transition hover:-translate-y-px">Follow one keystroke</a>
    </div>
    <div className="flex flex-col gap-2">
      ${LAYERS.map((l, i) => html`
        <${React.Fragment} key=${i}>
          <div className=${CARD + " flex items-center gap-3.5 px-4 py-3 " + (l.core ? "border-l-4 border-l-brand-600" : "")}>
            <span className="w-24 flex-none text-[.66rem] font-extrabold uppercase tracking-[.08em] text-slate-400">${l.tag}</span>
            <div>
              <div className="text-sm font-semibold">${l.t}</div>
              <div className="text-[.85rem] leading-snug text-slate-500">${l.d}</div>
            </div>
          </div>
          ${i < LAYERS.length - 1 ? html`<div className="text-center text-xs text-slate-300">▼</div>` : null}
        <//>`)}
    </div>
  </section>`;
