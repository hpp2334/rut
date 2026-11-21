/* §Lab — the completion playground: dotted code on the left, the
 * tier's answer on the right. */
import { html, useState } from "./vdom.js";
import { CARD, GROUP_H, Section, CodeWithDots, EdgeChip, Kind, Note, Plain } from "./ui.js";
import { LAB, LAB_CODE } from "./data.js";

const ItemRow = ({ it }) => html`
  <div className="flex items-center gap-2.5 rounded-lg border border-transparent px-2.5 py-1.5 hover:border-slate-200 hover:bg-slate-50">
    <span className="font-mono text-[.88rem] font-semibold">${it.label}</span>
    ${it.kind ? html`<${Kind} k=${it.kind} />` : null}
    ${it.insert ? html`<span className="whitespace-nowrap rounded border border-amber-200 bg-amber-50 px-1.5 py-px font-mono text-[.68rem] text-amber-700">+ ${it.insert}</span>` : null}
    ${it.detail ? html`<span className="ml-auto max-w-[55%] truncate text-right font-mono text-xs text-slate-400">${it.detail}</span>` : null}
  </div>`;

export const Lab = () => {
  const [sel, setSel] = useState("member");
  const hs = LAB.find((h) => h.id === sel);
  return html`<${Section} id="lab" num="05" title="Completion lab"
      kicker="One probe decides what kind of position the cursor is on — the edge — and each edge has its own tier. Click the dots in the code; the panel shows exactly what the server would answer.">
    <div className="mt-1 grid items-start gap-4 lg:grid-cols-[1.05fr_.95fr]">
      <${CodeWithDots} fname="inventory.rut" code=${LAB_CODE}
        hotspots=${LAB.map((h) => ({ id: h.id, ln: h.ln, after: h.after, title: h.title }))}
        sel=${sel} onPick=${setSel} />
      <div className=${CARD + " flex flex-col p-5 pt-4"}>
        <div className="flex flex-wrap items-center gap-2.5">
          <${EdgeChip} e=${hs.edge} />
          <span className="font-bold">${hs.title}</span>
        </div>
        <div className="mt-2 mb-2 max-h-[560px] overflow-y-auto pr-1">
          <p className="mt-1 mb-1.5 text-[.93rem] leading-relaxed text-slate-500">${hs.why}</p>
          ${hs.items ? hs.items.map((it, i) => html`<${ItemRow} key=${i} it=${it} />`) : null}
          ${hs.groups ? hs.groups.map(([g, items]) => html`
            <div key=${g}><div className=${GROUP_H}>${g}</div>
              ${items.map((it, i) => html`<${ItemRow} key=${i} it=${it} />`)}</div>`) : null}
          ${hs.note ? html`<${Note}>${hs.note}<//>` : null}
        </div>
      </div>
    </div>
    <${Plain}>Completion is three moves: <b>the edge probe</b> (a tree walk — which position
    kind is this?), <b>the tier door</b> (enumerate + gate, living in the collection), and
    <b>one conversion</b> at the ABI edge where a row becomes a wire item. Prefix filtering is
    presentation — it happens last, at the call site.<//>
    <table className="mt-4 w-full overflow-hidden rounded-xl border-collapse shadow-card text-sm [&_td]:border [&_td]:border-slate-200 [&_td]:p-2 [&_td]:align-top [&_th]:border [&_th]:border-slate-200 [&_th]:bg-slate-50 [&_th]:p-2 [&_th]:text-left [&_th]:text-xs [&_tbody_tr:hover]:bg-slate-50">
      <thead><tr><th>edge (tree walk)</th><th>recognized as</th><th>example</th></tr></thead>
      <tbody>
        <tr><td className="font-mono">Use</td><td className="text-slate-500">a use-node ancestor (uses are items)</td><td className="font-mono">use json::⏐</td></tr>
        <tr><td className="font-mono">Member</td><td className="text-slate-500">a method/field node — the absent-member shape included</td><td className="font-mono">c.⏐</td></tr>
        <tr><td className="font-mono">Qualified</td><td className="text-slate-500">a path's tail segment, or past a dangling <code>::</code></td><td className="font-mono">Color::⏐</td></tr>
        <tr><td className="font-mono">TypeDot</td><td className="text-slate-500">the type slot's member request</td><td className="font-mono">let c: layout.Co⏐</td></tr>
        <tr><td className="font-mono">Annotation</td><td className="text-slate-500">the <code>:</code> / <code>-&gt;</code> boundary</td><td className="font-mono">let x: ⏐</td></tr>
        <tr><td className="font-mono">Bare</td><td className="text-slate-500">anything else, with the partial ident</td><td className="font-mono">enc⏐</td></tr>
      </tbody>
    </table>
  <//>`;
};
