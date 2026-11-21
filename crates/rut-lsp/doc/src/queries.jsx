/* §Queries — the request-family cards plus the live hover demo. */
import { useState } from "react";
import { CARD, Section, Code, Icon } from "./ui.jsx";
import { tokenizeRut, Tok } from "./highlight.jsx";
import { QUERIES, HOVER_CODE, HOVER_TARGETS } from "./data.jsx";

const HoverCard = ({ card }) => (
  <div className="mx-auto mt-4 max-w-md rounded-xl border border-[#33436b] bg-[#1e2a44] px-4 py-3 shadow-[0_12px_32px_-8px_rgba(10,16,34,.5)]">
    <pre className="m-0 font-mono text-[13px] text-slate-200">{card.pre}</pre>
    {card.where ? <div className="mt-2 text-xs text-slate-400">{card.where}</div> : null}
    {card.doc ? <div className="mt-1 font-mono text-xs italic text-slate-500">// {card.doc}</div> : null}
  </div>
);

const HoverDemo = () => {
  const [sel, setSel] = useState(2);
  const card = HOVER_TARGETS[sel];
  return (
    <div className={CARD + " flex flex-col gap-2 p-5"}>
      <div className="font-bold">Hover, tried live</div>
      <p className="m-0 text-sm text-slate-500">Click any <span className="border-b-2 border-dotted border-indigo-400">underlined</span> name — that is a hover request at that position.</p>
      <Code fname="point.rut">
        {HOVER_CODE.split("\n").map((ln, i) => (
          <span key={i}>
            {tokenizeRut(ln).map((tk, j) => {
              const tgt = HOVER_TARGETS.find((t) => t.ln === i + 1 && t.tok === tk.v);
              if (tgt) {
                const idx = HOVER_TARGETS.indexOf(tgt);
                return (
                  <button key={j}
                    className={"cursor-pointer rounded-sm border-b-2 border-dotted border-[#82aaff] " +
                      (sel === idx ? "bg-[#82aaff]/25" : "hover:bg-[#82aaff]/15")}
                    onClick={() => setSel(idx)}>{tk.v}</button>
                );
              }
              return <Tok key={j} tk={tk} />;
            })}
            {"\n"}
          </span>
        ))}
      </Code>
      <HoverCard card={card} />
      <div className="text-center text-[.82rem] text-slate-400">{card.label} — the card is the declaration's own text, sliced verbatim.</div>
    </div>
  );
};

export const Queries = () => (
  <Section id="queries" num="04" title="The queries"
    kicker="One module per request family. Each consumes the chain and answers LSP-shaped JSON — a miss is null, never wrong text.">
    <div className="grid gap-3.5 md:grid-cols-2">
      {QUERIES.map((q) => (
        <div className={CARD + " flex flex-col gap-2 p-5"} key={q.name}>
          <div className="flex items-center gap-2.5">
            <span className="inline-flex h-9 w-9 flex-none items-center justify-center rounded-xl bg-indigo-50 text-indigo-600"><Icon d={q.icon} /></span>
            <div><div className="font-bold">{q.name}</div>
            <code className="text-xs text-slate-500">{q.sig}</code></div>
          </div>
          <p className="m-0 text-sm leading-relaxed text-slate-500">{q.d}</p>
          <div className="overflow-x-auto whitespace-pre rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 font-mono text-xs text-slate-600">{q.ex}</div>
        </div>
      ))}
      <HoverDemo />
    </div>
  </Section>
);
