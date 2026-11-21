/* §Pipeline — the keystroke stepper: type → diff → incremental repair
 * → one coherent document. The step bodies carry their own visuals
 * (diff boxes), so the STEPS data lives here, beside them. */
import { html, useState } from "./vdom.js";
import { CARD, Section, Note, Plain } from "./ui.js";

const DiffBox = ({ cap, add, children }) => html`
  <div className=${"whitespace-pre-wrap rounded-xl bg-slate-100 px-4 py-2.5 font-mono text-[12.8px] leading-[1.7] text-slate-700 " +
    (add ? "border border-emerald-200 bg-emerald-50/60" : "")}>
    <span className="mb-1.5 block font-sans text-[.66rem] font-extrabold uppercase tracking-[.09em] text-slate-400">${cap}</span>${children}
  </div>`;
const Hl = ({ children }) => html`<span className="rounded bg-amber-200/80 px-0.5">${children}</span>`;

const STEPS = [
  {
    t: "You type",
    d: html`<p className="mt-1 mb-4">The editor holds the whole text and reports the whole
      new text on every keystroke (full-text sync). Say you finish typing a field access:</p>
      <div className="grid gap-3 sm:grid-cols-2">
        <${DiffBox} cap="before">  return <${Hl}>p.</${Hl}>;<//>
        <${DiffBox} cap="after" add=${true}>  return <${Hl}>p.x</${Hl}>;<//>
      </div>
      <p className="mt-3 text-sm text-slate-500">Everything the server knows arrives as this one
      string — no ranges, no incremental protocol. The server makes it incremental itself.</p>`,
  },
  {
    t: "Diff → one Edit",
    d: html`<p className="mt-1 mb-4"><code className="rounded bg-slate-200/70 px-1.5 py-0.5 text-[.9em]">Document::sync</code> diffs old
      against new — common prefix, common suffix — and snaps to character boundaries. One tiny value describes the change:</p>
      <div className="grid gap-3 sm:grid-cols-2">
        <${DiffBox} cap="the edit">Edit { start: <${Hl}>11</${Hl}>, old_end: 11, text: <${Hl}>"x"</${Hl}> }<//>
        <${DiffBox} cap="what it means">insert <${Hl}>x</${Hl}> at byte 11 — nothing deleted<//>
      </div>
      <p className="mt-3 text-sm text-slate-500">The wire shell (protocol) adapts to this shape —
      the diff never happens below the adapter.</p>`,
  },
  {
    t: "Incremental repair",
    d: html`<p className="mt-1 mb-3"><code className="rounded bg-slate-200/70 px-1.5 py-0.5 text-[.9em]">Doc::edit</code> — the one and only
      mutator — repairs the pipeline stage by stage, reusing everything the edit didn't touch:</p>
      <div className="flex flex-col gap-2">
        ${[
          ["text", "the source splices the string in, line geometry maintained"],
          ["tokens", "relex only a window: from a line boundary outside any literal, until the stream converges with the old tail — then splice + shift"],
          ["tree", "find the dirty root item (the reuse unit), reparse just that window into fresh slots of the live store"],
          ["splice", "the new subtree lands under the surviving parent; dead slots punch holes; the index invalidates (it rebuilds lazily on next read)"],
        ].map(([a, b], i) => html`
          <${React.Fragment} key=${i}>
            <div className=${CARD + " flex items-center gap-3.5 px-4 py-2.5"}>
              <span className="w-20 flex-none text-[.66rem] font-extrabold uppercase tracking-[.08em] text-slate-400">${a}</span>
              <div className="text-[.85rem] text-slate-500">${b}</div>
            </div>
            ${i < 3 ? html`<div className="text-center text-xs text-slate-300">▼</div>` : null}
          <//>`)}
      </div>`,
  },
  {
    t: "One coherent document",
    d: html`<p className="mt-1 mb-3">The UTF-16 line view rebuilds and the lazy views
      (semantic tokens, symbols, inlays) drop — the first reader after the change re-derives them.
      The burst of queries riding one version re-derives <b>nothing</b>.</p>
      <${Note}>The incremental lane is pinned <b>edit ≡ cold</b> by property tests: random edit
      sequences must produce exactly the tokens, spans, tree, errors and index a from-scratch parse
      would. You cannot drift the fast path from the correct one.<//>`,
  },
];

export const Pipeline = () => {
  const [sel, setSel] = useState(0);
  return html`<${Section} id="pipeline" num="01" title="Follow one keystroke"
      kicker="An editor change becomes a coherent, incrementally-repaired document — parse once per change, never per query.">
    <div className=${CARD + " p-5"}>
      <div className="mb-4 flex flex-wrap gap-2">
        ${STEPS.map((s, i) => html`<button key=${i}
          className=${"flex cursor-pointer items-center gap-2.5 rounded-full border px-4 py-2 text-sm font-semibold transition " +
            (sel === i ? "border-slate-900 text-slate-900 shadow-card" : "border-slate-200 bg-white text-slate-500 hover:border-slate-300")}
          onClick=${() => setSel(i)}>
          <span className=${"flex h-[22px] w-[22px] items-center justify-center rounded-full text-[.72rem] font-extrabold " +
            (sel === i ? "bg-brand-600 text-white" : "bg-slate-100 text-slate-500")}>${i + 1}</span>${s.t}
        </button>`)}
      </div>
      <div className="animate-fade-up" key=${sel}>${STEPS[sel].d}</div>
    </div>
    <${Plain}>Think of it as surgery instead of re-birth: the document keeps its identity, and only
    the region your keystroke touched is rebuilt. Everything the edit missed — every other item,
    every other function — is byte-identical and simply stays.<//>
  <//>`;
};
