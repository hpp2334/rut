/* §Wire — the envelope examples, the three ABI lanes, the HostEnv. */
import { CARD, Section, Rus, Law, Note } from "./ui.jsx";
import { LANES, WIRE_JS, HOSTENV_TS } from "./data.jsx";

export const Wire = () => (
  <Section id="wire" num="06" title="The wire & the ABI"
    kicker="Everything crosses as JSON strings — one exported class, three lanes, a read-only environment. No internal type can ever leak.">
    <Rus fname="boot.ts" code={WIRE_JS} />
    <div className="my-4 grid gap-3.5 md:grid-cols-3">
      {LANES.map((l) => (
        <div className={CARD + " p-5"} key={l.t}>
          <div className="mb-1.5"><span className="rounded-lg border border-indigo-200 bg-indigo-50 px-2.5 py-0.5 text-xs font-bold text-indigo-600">{l.t}</span></div>
          <p className="m-0 text-sm leading-relaxed text-slate-500">{l.d}</p>
        </div>
      ))}
    </div>
    <Rus fname="HostEnv.d.ts" code={HOSTENV_TS} />
    <Law>The environment grant is read-only <b>by construction</b>: no write, delete or fetch
    exists on the trait, so no handler can spell one. The clock is the one machinery addition —
    the host supplies the timer, but the actor owns every when. Toward the host, the actor only
    ever answers.</Law>
    <Note>Url dependencies mount from the local content-addressed store; ones the walk
    cannot fetch become missing rows plus a once-per-session hint: <i>run rut install</i>.
    The rust-analyzer contract.</Note>
  </Section>
);
