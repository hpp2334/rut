/**
 * The playground's rut-lsp adapter.
 *
 * The binding itself — the class that parses the wasm's JSON envelopes,
 * its types, and the semantic-token decoder — lives in the wasm crate
 * (`crates/rut-lsp-wasm/js`), one face shared with the VS Code
 * extension. This module adds only what a browser needs on top: a
 * same-origin fetch of `public/rut-lsp.wasm`, the playground's own
 * document URI, and the dep-lane methods the deps panel drives
 * (`parseManifest` / `resolveDeps` — the walk reads the virtual
 * `$RUT_HOME` through the injected `HostEnv`, see `store-env.ts`). No
 * LSP shape is declared here.
 *
 * The artifact is built from crates/rut-lsp-wasm by `npm run build:wasm`
 * (the same command that builds rut-wasm). The module is CONSUMED here,
 * never patched: a gap found through this binding is recorded upstream,
 * not worked around.
 *
 * The playground's own slice: `analyze(uri, src)` — the doc is stored
 * full-sync and the ONE response carries diagnostics AND the full
 * semantic-token stream (five u32s per token, legend names via
 * `legend()`) — plus `complete` at the caret (the editor's popup) and
 * the dep walk after boot/edits/custom adds.
 */

import { RutLsp as Binding, type Analysis, type HostEnv } from "../../../crates/rut-lsp-wasm/pkg/index.js";

export { decodeTokens } from "../../../crates/rut-lsp-wasm/pkg/index.js";

// the types the playground's own modules read — re-exported here so no
// app file has to reach into the crate directly
export type {
  Analysis,
  DecodedToken,
  DepTable,
  Envelope,
  HostEnv,
  LspCompletionItem,
  LspDiag,
  LspPosition,
  LspRange,
  WalkOut,
} from "../../../crates/rut-lsp-wasm/pkg/index.js";

export { isEnvelope } from "../../../crates/rut-lsp-wasm/pkg/index.js";

export { PLAYGROUND_DOC } from "./store-env";

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
   * headless smoke reads the same file from disk); `env` — the
   * read-only host environment the dep walk reads through (the
   * virtual `$RUT_HOME` + workspace) — rides `load`'s own env lane.
   * Everything after the bytes is the one shared path the browser
   * uses. Throws on any failure; the App maps that onto the phase-1
   * error-panel shape. */
  static async boot(
    load?: (url: string) => Promise<ArrayBuffer>,
    env?: HostEnv,
  ): Promise<RutLsp> {
    const bytes = await (load ?? fetchBytes)("rut-lsp.wasm");
    return new RutLsp(await Binding.load(new Uint8Array(bytes), env));
  }

  /** token-type names, in legend index order (the CSS map keys on
   * these names, never on a hardcoded order). Async — the actor seam:
   * every bridge call posts to the core's event loop and awaits its
   * reply. */
  get legend(): Promise<string[]> {
    return this.core.legend();
  }

  /** open / FULL-sync change: store the doc, get diags + tokens in the
   * one response (the survey's call) — async like every bridge */
  async analyze(uri: string, src: string): Promise<Analysis> {
    return this.core.analyze(uri, src);
  }

  /** completions at an LSP position over the doc's CURRENT text (the
   * doc-sync law: the source rides the call) — the editor popup's
   * query */
  async complete(
    uri: string,
    line: number,
    character: number,
    src: string,
  ): Promise<import("../../../crates/rut-lsp-wasm/pkg/index.js").LspCompletionItem[]> {
    return this.core.complete(uri, line, character, src);
  }

  /** parse one `rut.jsonc` — the dep table, or the error envelope */
  parseManifest(text: string) {
    return this.core.parseManifest(text);
  }

  /** the READ-ONLY dep walk: mounts the store's world into the defs
   * store; the answer carries the refused url rows (the deps panel's
   * per-dep status) */
  resolveDeps(roots: string[]) {
    return this.core.resolveDeps(roots);
  }
}
