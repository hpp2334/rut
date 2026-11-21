/* Shared UI atoms: class strings, kind chips, callouts, the code
 * cards (plain, highlighted, dot-marked), icons, and the nav +
 * section frame every page block rides. */
import { html, useState, useEffect } from "./vdom.js";
import { tokenizeRut, Tok, splitTokens } from "./highlight.js";

/* shared class strings ------------------------------------------------ */
export const CARD = "rounded-2xl border border-slate-200 bg-white shadow-card";
export const CODE = "overflow-hidden rounded-xl bg-[#0d1526] shadow-card";
export const PRE = "m-0 overflow-x-auto px-5 py-4 font-mono text-[13.5px] leading-[1.65] text-slate-200";
export const GROUP_H = "mt-4 mb-1.5 text-[.68rem] font-extrabold uppercase tracking-[.09em] text-slate-400";
export const KICKER = "mt-2 mb-7 max-w-[46em] text-[1.02rem] leading-relaxed text-slate-500";

const KIND_CLS = {
  Keyword: "bg-slate-100 text-slate-600",
  Field: "bg-emerald-50 text-emerald-700",
  Method: "bg-indigo-50 text-indigo-700",
  Fn: "bg-blue-50 text-blue-700",
  Type: "bg-violet-50 text-violet-700",
  Interface: "bg-purple-50 text-purple-700",
  EnumMember: "bg-amber-50 text-amber-700",
  Variable: "bg-teal-50 text-teal-700",
  Parameter: "bg-teal-50 text-teal-700",
  Const: "bg-yellow-50 text-yellow-700",
  Module: "bg-cyan-50 text-cyan-700",
};

export const Kind = ({ k }) => html`<span className=${"kind " + (KIND_CLS[k] || "")}>${k}</span>`;
export const EdgeChip = ({ e }) => html`<span className="kind border border-brand-200 bg-brand-50 text-brand-600">${"edge: " + e}</span>`;

/* callouts ------------------------------------------------------------- */
const CODE_CALLOUT = {
  plain: { cls: "border-amber-200 bg-amber-50", tag: "text-amber-700" },
  law: { cls: "border-brand-200 bg-brand-50", tag: "text-brand-600" },
  note: { cls: "border-indigo-200 bg-indigo-50", tag: "text-indigo-600" },
};
const CALLOUT_LABEL = { plain: "in plain terms", law: "the law", note: "note" };
const Callout = ({ kind, children }) => html`
  <div className=${"mt-4 mb-4 flex items-start gap-3 rounded-xl border px-4 py-3 text-[.95rem] " + CODE_CALLOUT[kind].cls}>
    <span className=${"whitespace-nowrap pt-0.5 text-[.7rem] font-extrabold uppercase tracking-wider " + CODE_CALLOUT[kind].tag}>${CALLOUT_LABEL[kind]}</span>
    <div className="[&>b]:font-bold [&>code]:rounded [&>code]:bg-slate-200/70 [&>code]:px-1.5 [&>code]:py-0.5 [&>code]:text-[.85em] [&>i]:italic">${children}</div>
  </div>`;
export const Plain = ({ children }) => html`<${Callout} kind="plain">${children}<//>`;
export const Law = ({ children }) => html`<${Callout} kind="law">${children}<//>`;
export const Note = ({ children }) => html`<${Callout} kind="note">${children}<//>`;

/* code cards ------------------------------------------------------------ */
export const Code = ({ fname, children }) => html`
  <div className=${CODE + " my-4"}>
    <div className="flex items-center gap-2 border-b border-white/10 bg-white/5 px-4 py-2">
      <span className="flex gap-1.5">
        ${["#ff5f57", "#febc2e", "#28c840"].map((c) => html`<span key=${c}
          className="h-2.5 w-2.5 rounded-full opacity-85" style=${{ background: c }} />`)}
      </span>
      <span className="font-mono text-xs text-slate-400">${fname || "snippet.rut"}</span>
    </div>
    <pre className=${PRE}><code>${children}</code></pre>
  </div>`;

export const Rus = ({ fname, code }) => html`<${Code} fname=${fname}>
  ${code.split("\n").map((ln, i) => html`<span key=${i}>
    ${ln === "" ? " " : tokenizeRut(ln).map((tk, j) => html`<${Tok} key=${j} tk=${tk} />`)}
    ${"\n"}
  </span>`)}
<//>`;

/* code with clickable cursor dots (anchor = "dot sits right after this text") */
export const CodeWithDots = ({ fname, code, hotspots, sel, onPick }) =>
  html`<${Code} fname=${fname}>
    ${code.split("\n").map((ln, i) => {
      const hs = hotspots.find((h) => h.ln === i + 1);
      let head = null, tail = null;
      if (hs) {
        const at = ln.indexOf(hs.after) + hs.after.length;
        [head, tail] = splitTokens(tokenizeRut(ln), at);
      }
      return html`<span key=${i}>
        ${hs
          ? html`${head.map((tk, j) => html`<${Tok} key=${j} tk=${tk} />`)}
             <button title=${hs.title} aria-label=${hs.title}
               className=${"mx-px cursor-pointer text-[.82em] " + (sel === hs.id
                 ? "text-orange-400 [text-shadow:0_0_12px_rgba(255,157,46,.9)]"
                 : "animate-dot-pulse text-amber-400 hover:text-amber-300")}
               onClick=${() => onPick(hs.id)}>●</button>
             ${tail.map((tk, j) => html`<${Tok} key=${"t" + j} tk=${tk} />`)}`
          : ln === "" ? " " : tokenizeRut(ln).map((tk, j) => html`<${Tok} key=${j} tk=${tk} />`)}
        ${"\n"}
      </span>`;
    })}
  <//>`;

/* icons ------------------------------------------------------------------ */
const ICONS = {
  pulse: "M3 12h4l3-8 4 16 3-8h4",
  search: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zm9 16-4.5-4.5",
  arrow: "M5 12h14m-6-6 6 6-6 6",
  link: "M10 14a5 5 0 0 0 7.07 0l2.83-2.83a5 5 0 0 0-7.07-7.07L11.5 5.4M14 10a5 5 0 0 0-7.07 0L4.1 12.83a5 5 0 0 0 7.07 7.07l1.33-1.3",
  tag: "M20.6 13.4 13.4 20.6a2 2 0 0 1-2.8 0l-7.2-7.2V3.4h10l7.2 7.2a2 2 0 0 1 0 2.8zM7.8 7.8h.01",
  parens: "M8 4C5.5 6 4 9 4 12s1.5 6 4 8M16 4c2.5 2 4 5 4 8s-1.5 6-4 8",
  spark: "M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9zM19 17l.8 2.2L22 20l-2.2.8L19 23l-.8-2.2L16 20l2.2-.8z",
  box: "M21 8l-9-5-9 5v8l9 5 9-5V8zM3 8l9 5 9-5M12 13v8",
};
export const Icon = ({ d, size = 17, cls = "" }) => html`
  <svg className=${cls} width=${size} height=${size} viewBox="0 0 24 24" fill="none"
       stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <path d=${ICONS[d]} /></svg>`;

/* nav + section frame ------------------------------------------------------ */
export const SECTIONS = [
  { id: "overview", label: "Overview" },
  { id: "pipeline", label: "Typing pipeline" },
  { id: "world", label: "Session & world" },
  { id: "member", label: "The member law" },
  { id: "queries", label: "The queries" },
  { id: "lab", label: "Completion lab" },
  { id: "wire", label: "Wire & ABI" },
  { id: "map", label: "Code map" },
];

function useActiveSection() {
  const [active, setActive] = useState(SECTIONS[0].id);
  useEffect(() => {
    const obs = new IntersectionObserver((entries) => {
      const vis = entries.filter((e) => e.isIntersecting)
        .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
      if (vis[0]) setActive(vis[0].target.id);
    }, { rootMargin: "-90px 0px -65% 0px", threshold: 0 });
    SECTIONS.forEach((s) => { const el = document.getElementById(s.id); if (el) obs.observe(el); });
    return () => obs.disconnect();
  }, []);
  return active;
}

export const Nav = () => {
  const active = useActiveSection();
  return html`
    <div className="sticky top-0 z-50 border-b border-slate-200/80 bg-white/85 backdrop-blur-md">
      <nav className="mx-auto flex max-w-[1160px] items-center gap-5 px-6 py-2.5">
        <span className="flex items-baseline gap-2 whitespace-nowrap text-base font-extrabold">
          <span className="text-brand-600">●</span>rut-lsp
          <small className="hidden text-[.7rem] font-semibold text-slate-400 sm:inline">the language server, explained</small>
        </span>
        <div className="flex flex-1 gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
          ${SECTIONS.map((s) => html`<a key=${s.id} href="#${s.id}"
            className=${"whitespace-nowrap rounded-full px-3 py-1.5 text-[.85rem] font-medium " +
              (active === s.id ? "bg-slate-900 text-white" : "text-slate-500 hover:bg-slate-100")}>${s.label}</a>`)}
        </div>
      </nav>
    </div>`;
};

export const Section = ({ id, num, title, kicker, children }) => html`
  <section className="pt-11" id=${id}>
    <div className="flex items-baseline gap-3.5">
      <span className="text-xs font-extrabold tracking-[.12em] text-brand-600">${num}</span>
      <h2 className="text-[1.7rem] font-bold tracking-tight">${title}</h2>
    </div>
    <p className=${KICKER}>${kicker}</p>
    ${children}
  </section>`;
