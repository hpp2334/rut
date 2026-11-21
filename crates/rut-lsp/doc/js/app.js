/* The app root: nav + every section in scroll order, mounted once. */
import { html } from "./vdom.js";
import { Nav } from "./ui.js";
import { Overview } from "./overview.js";
import { Pipeline } from "./pipeline.js";
import { World } from "./world.js";
import { MemberLaw } from "./member-law.js";
import { Queries } from "./queries.js";
import { Lab } from "./lab.js";
import { Wire } from "./wire.js";
import { CodeMap } from "./codemap.js";

const App = () => html`
  <${Nav} />
  <main className="mx-auto max-w-[1160px] px-6 pb-24">
    <${Overview} />
    <${Pipeline} />
    <${World} />
    <${MemberLaw} />
    <${Queries} />
    <${Lab} />
    <${Wire} />
    <${CodeMap} />
    <footer className="mt-16 border-t border-slate-200 pt-7 text-sm text-slate-400">
      rut-lsp interactive doc · <code className="font-mono">crates/rut-lsp/doc/</code> · React + htm + Tailwind
      from jsDelivr, ES modules, no build step. Examples mirror the crate's unit-test fixtures
      (hover.rs, completion.rs) — what you click is what the tests pin. Keep beside AGENTS.md's
      "Frontend layering" laws when the architecture moves.
    </footer>
  </main>`;

ReactDOM.createRoot(document.getElementById("root")).render(html`<${App} />`);
