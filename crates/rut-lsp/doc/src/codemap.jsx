/* §CodeMap — the file table, the laws accordion, the gates. Named
 * CodeMap, never Map: a module-level `const Map` would shadow the
 * builtin at a glance. */
import { CARD, Section } from "./ui.jsx";
import { FILES, LAWS, GATES } from "./data.jsx";

export const CodeMap = () => (
  <Section id="map" num="07" title="Code map"
    kicker="Where everything lives, the laws in one breath, and the gates that keep them true.">
    <table className="w-full overflow-hidden rounded-xl border-collapse shadow-card text-sm [&_td]:border [&_td]:border-slate-200 [&_td]:p-2 [&_td]:align-top [&_th]:border [&_th]:border-slate-200 [&_th]:bg-slate-50 [&_th]:p-2 [&_th]:text-left [&_th]:text-xs">
      <thead><tr><th>path</th><th>what lives there</th></tr></thead>
      <tbody>
        {FILES.map(([a, b]) => (
          <tr key={a}>
            <td className="whitespace-nowrap font-mono text-xs">{a}</td>
            <td className="text-slate-500">{b}</td>
          </tr>
        ))}
      </tbody>
    </table>
    <div className="mt-3">
      {LAWS.map(([a, b]) => (
        <details key={a} className={CARD + " my-2 py-1 [&_.inner]:px-5 [&_.inner]:pb-3.5 [&_.inner]:text-sm [&_.inner]:text-slate-500"}>
          <summary className="flex cursor-pointer list-none items-center gap-2.5 px-5 py-3 font-bold [&::-webkit-details-marker]:hidden">
            <span className="font-extrabold text-brand-600">+</span>{a}</summary>
          <div className="inner">{b}</div>
        </details>
      ))}
    </div>
    <div className={CARD + " mt-4 p-5"}>
      <div className="mb-2 font-bold">The gates</div>
      <ul className="m-0 list-disc pl-5 text-sm text-slate-500">
        {GATES.map((g) => <li key={g} className="my-1">{g}</li>)}
      </ul>
    </div>
  </Section>
);
