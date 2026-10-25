/**
 * The rut-lsp-wasm binding — the demo's typed twin of the extension's
 * `src/wasm.ts` (survey D3). The wasm module is the rut-lsp-wasm
 * crate's `RutLsp` class over wasm-bindgen: the glue
 * (crates/rut-lsp-wasm/pkg/rut_lsp_wasm.js, gitignored build output) is
 * bundled here; `boot` fetches the artifact bytes and hands them to the
 * glue's `init`, and every answer is the JSON envelope as a string,
 * parsed here. Single-threaded JS, one `RefCell` borrow per call: the
 * instance owns its session state (the open doc + the index).
 *
 * The artifact is `public/rut-lsp.wasm` (gitignored), built from
 * crates/rut-lsp-wasm by `npm run build:wasm` (one command, both
 * artifacts). The module is CONSUMED here, never patched: a gap found
 * through this binding is recorded upstream, not worked around.
 *
 * The demo's slice is one call: `analyze(uri, src)` — the doc is
 * stored full-sync and the ONE response carries diagnostics AND the
 * full semantic-token stream (five u32s per token, legend names via
 * `legend()`). Doc-sized sources re-analyze in milliseconds, so the
 * editor debounces ~150 ms and re-sends the whole doc per change.
 */

import init, { RutLsp as WasmCore } from "../../../crates/rut-lsp-wasm/pkg/rut_lsp_wasm.js";

/** the single playground document uri — full-sync overwrite per change */
export const PLAYGROUND_DOC = "file:///playground/main.rut";

export interface LspPosition {
  line: number;
  character: number;
}

export interface LspRange {
  start: LspPosition;
  end: LspPosition;
}

/** an LSP diagnostic — the while-typing surface (squiggles, survey D3) */
export interface LspDiag {
  range: LspRange;
  severity?: number;
  message: string;
}

/** the analyze response: diags + the flat SemanticTokens.data array */
export interface LspAnalysis {
  diags: LspDiag[];
  tokens: { data: number[] };
}

/** one decoded token — absolute 0-based LSP coordinates */
export interface DecodedToken {
  line: number;
  start: number;
  length: number;
  /** index into `legend()` — never a hardcoded position */
  type: number;
}

/** the browser byte source: same-origin fetch of the shipped artifact */
async function fetchBytes(url: string): Promise<ArrayBuffer> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`fetch ${url}: HTTP ${res.status}`);
  return res.arrayBuffer();
}

export class RutLsp {
  private core: WasmCore;
  private legendNames: string[];

  private constructor(core: WasmCore) {
    this.core = core;
    // the artifact and the page ship together — init succeeded, so the
    // glue matches the module; the legend read proves the surface live
    this.legendNames = JSON.parse(core.legend());
  }

  /** Load the module once. `load` overrides the byte source (the
   * headless smoke reads the same file from disk); everything after the
   * bytes — init, the legend read — is the one shared path the browser
   * uses. Throws on any failure; the App maps that onto the phase-1
   * error-panel shape. */
  static async boot(load?: (url: string) => Promise<ArrayBuffer>): Promise<RutLsp> {
    const bytes = await (load ?? fetchBytes)("rut-lsp.wasm");
    await init({ module_or_path: new Uint8Array(bytes) });
    return new RutLsp(new WasmCore());
  }

  /** token-type names, in legend index order (the CSS map keys on
   * these names, never on a hardcoded order) */
  get legend(): string[] {
    return this.legendNames;
  }

  /** open / FULL-sync change: store the doc, get diags + tokens in the
   * one response (the survey's call) */
  analyze(uri: string, src: string): LspAnalysis {
    return JSON.parse(this.core.analyze(uri, src)) as LspAnalysis;
  }
}

/** Decode the flat `SemanticTokens.data` wire array (5 u32s per token:
 * deltaLine, deltaStart, length, type-index, modifiers) into absolute
 * positions. Shared by the editor's overlay builder and the smoke's
 * census — one decode, one truth. */
export function decodeTokens(data: number[]): DecodedToken[] {
  const out: DecodedToken[] = [];
  let line = 0;
  let col = 0;
  for (let i = 0; i + 4 < data.length; i += 5) {
    const deltaLine = data[i];
    const deltaStart = data[i + 1];
    if (deltaLine > 0) {
      line += deltaLine;
      col = deltaStart;
    } else {
      col += deltaStart;
    }
    out.push({ line, start: col, length: data[i + 2], type: data[i + 3] });
  }
  return out;
}
