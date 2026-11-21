/* §World — MountedFile's three parts, the Chain, and the doors. */
import { CARD, CODE, PRE, Section, Note, Plain } from "./ui.jsx";
import { DOORS } from "./data.jsx";

const CHAIN_PRE = `
  chain.head()    ▸ the OPEN document (freshest version, rebuilt on every sync)
  chain.iter()    ▸ then the world: prelude · walked deps · workspace files

  let chain = state.chain(uri)?;      // one ordering view per query
  chain.type_decl("Counter")          // doors — one home per question ↓`;

export const World = () => (
  <Section id="world" num="02" title="The session & the world"
    kicker="The server keeps one shape for every file it has ever seen — open tabs, the prelude, walked dependencies.">
    <div className="mb-4 grid gap-3.5 md:grid-cols-3">
      <div className={CARD + " p-5"}><div className="mb-1.5 font-bold"><code className="text-[.9em]">doc</code> — the kept tree</div>
        <p className="m-0 text-sm leading-relaxed text-slate-500">The full pipeline artifact: normalized text, tokens,
        the syntax tree. Parsed once per change, repaired incrementally.</p></div>
      <div className={CARD + " p-5"}><div className="mb-1.5 font-bold"><code className="text-[.9em]">mount</code> — the stamp</div>
        <p className="m-0 text-sm leading-relaxed text-slate-500">Provenance: origin uri, package name, module path,
        parse mode. Laid down at the mount site, untouched by edits.</p></div>
      <div className={CARD + " p-5"}><div className="mb-1.5 font-bold"><code className="text-[.9em]">decls</code> — the decl map</div>
        <p className="m-0 text-sm leading-relaxed text-slate-500">Node addresses in source order (types, fns, lets,
        uses, mods). The smallest possible index — everything else derives at read.</p></div>
    </div>
    <div className={CARD + " p-5"}>
      <div className="font-bold">The Chain — how a query sees the world</div>
      <div className={CODE + " my-3"}>
        <pre className={PRE + " text-slate-300"}>{CHAIN_PRE}</pre>
      </div>
      <table className="w-full overflow-hidden rounded-xl border-collapse shadow-card [&_td]:border [&_td]:border-slate-200 [&_td]:p-2 [&_td]:align-top [&_th]:border [&_th]:border-slate-200 [&_th]:bg-slate-50 [&_th]:p-2 [&_th]:text-left [&_th]:text-xs [&_tbody_tr:hover]:bg-slate-50">
        <tbody>
          {DOORS.map(([a, b]) => (
            <tr key={a}>
              <td className="whitespace-nowrap font-mono text-[.82rem]">{a}</td>
              <td className="text-sm text-slate-500">{b}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
    <Plain>The open document is just another file in the world — only fresher, and always first in
    line. There is no separate "current buffer" data structure to fall out of sync.</Plain>
    <Note>Cross-file references and go-to-definition jump into mounted dependency files without
    re-parsing them — the walk already left their trees in the world.</Note>
  </Section>
);
