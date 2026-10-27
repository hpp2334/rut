/**
 * The playground's rut-lsp adapter.
 *
 * The binding itself — the class that parses the wasm's JSON envelopes,
 * its types, and the semantic-token decoder — lives in the wasm crate
 * (`crates/rut-lsp-wasm/js`), one face shared with the VS Code
 * extension. This module adds only what a browser needs on top: a
 * same-origin fetch of `public/rut-lsp.wasm` and the playground's own
 * document URI. No LSP shape is declared here.
 *
 * The artifact is built from crates/rut-lsp-wasm by `npm run build:wasm`
 * (the same command that builds rut-wasm). The module is CONSUMED here,
 * never patched: a gap found through this binding is recorded upstream,
 * not worked around.
 *
 * The playground's slice is one call: `analyze(uri, src)` — the doc is
 * stored full-sync and the ONE response carries diagnostics AND the
 * full semantic-token stream (five u32s per token, legend names via
 * `legend()`). Doc-sized sources re-analyze in milliseconds, so the
 * editor debounces ~150 ms and re-sends the whole doc per change.
 */

import { RutLsp as Binding, type Analysis } from "../../../crates/rut-lsp-wasm/pkg/index.js";

export { decodeTokens } from "../../../crates/rut-lsp-wasm/pkg/index.js";

// the types the playground's own modules read — re-exported here so no
// app file has to reach into the crate directly
export type {
  Analysis,
  DecodedToken,
  LspDiag,
  LspPosition,
  LspRange,
} from "../../../crates/rut-lsp-wasm/pkg/index.js";

/** the single playground document uri — full-sync overwrite per change */
export const PLAYGROUND_DOC = "file:///playground/main.rut";

/** the browser byte source: same-origin fetch of the shipped artifact */
async function fetchBytes(url: string): Promise<ArrayBuffer> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`fetch ${url}: HTTP ${res.status}`);
  return res.arrayBuffer();
}

/** the playground's handle on the language core */
export class RutLsp {
  private constructor(private readonly core: Binding) {}

  /** Load the module once. `load` overrides the byte source (the
   * headless smoke reads the same file from disk); everything after the
   * bytes is the one shared path the browser uses. Throws on any
   * failure; the App maps that onto the phase-1 error-panel shape. */
  static async boot(load?: (url: string) => Promise<ArrayBuffer>): Promise<RutLsp> {
    const bytes = await (load ?? fetchBytes)("rut-lsp.wasm");
    return new RutLsp(await Binding.load(new Uint8Array(bytes)));
  }

  /** token-type names, in legend index order (the CSS map keys on
   * these names, never on a hardcoded order) */
  get legend(): string[] {
    return this.core.legend();
  }

  /** open / FULL-sync change: store the doc, get diags + tokens in the
   * one response (the survey's call) */
  analyze(uri: string, src: string): Analysis {
    return this.core.analyze(uri, src);
  }
}
