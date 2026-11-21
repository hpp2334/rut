/* The app root: nav + every section in scroll order. */
import { Nav } from "./ui.jsx";
import { Overview } from "./overview.jsx";
import { Pipeline } from "./pipeline.jsx";
import { World } from "./world.jsx";
import { MemberLaw } from "./member-law.jsx";
import { Queries } from "./queries.jsx";
import { Lab } from "./lab.jsx";
import { Wire } from "./wire.jsx";
import { CodeMap } from "./codemap.jsx";

export const App = () => (
  <>
    <Nav />
    <main className="mx-auto max-w-[1160px] px-6 pb-24">
      <Overview />
      <Pipeline />
      <World />
      <MemberLaw />
      <Queries />
      <Lab />
      <Wire />
      <CodeMap />
      <footer className="mt-16 border-t border-slate-200 pt-7 text-sm text-slate-400">
        rut-lsp interactive doc · <code className="font-mono">crates/rut-lsp/doc/</code> · React + Tailwind v4,
        bundled with rspack (npm packages, hashed assets). Examples mirror the crate's unit-test fixtures
        (hover.rs, completion.rs) — what you click is what the tests pin. Keep beside AGENTS.md's
        "Frontend layering" laws when the architecture moves.
      </footer>
    </main>
  </>
);
