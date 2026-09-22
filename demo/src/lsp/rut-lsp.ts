/**
 * The rut-lsp-wasm binding — the demo's typed twin of the extension's
 * `src/wasm.ts` (survey D3). Same raw ABI (RFC 0041 §2), same envelope:
 * inputs land in the per-request arena via `rut_alloc`, answers come
 * back as `[u32 LE length][JSON bytes]` at the returned pointer, and
 * every request opens with `rut_begin()` (the arena is per-request; the
 * module is long-lived). Single-threaded JS makes the protocol
 * trivially safe.
 *
 * The artifact is `public/rut-lsp.wasm` (gitignored), built from
 * crates/rut-lsp-wasm by `npm run build:wasm` (one command, both
 * artifacts). The module is CONSUMED here, never patched: a gap found
 * through this binding is recorded upstream, not worked around.
 *
 * The demo's slice is one call: `rut_analyze(uri, src)` — the doc is
 * stored full-sync and the ONE response carries diagnostics AND the
 * full semantic-token stream (five u32s per token, legend names via
 * `rut_legend()`). Doc-sized sources re-analyze in milliseconds, so the
 * editor debounces ~150 ms and re-sends the whole doc per change.
 */

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

interface LspExports {
  memory: WebAssembly.Memory;
  rut_begin(): void;
  rut_alloc(len: number): number;
  rut_legend(): number;
  rut_analyze(
    uriPtr: number,
    uriLen: number,
    srcPtr: number,
    srcLen: number,
  ): number;
}

/** the browser byte source: same-origin fetch of the shipped artifact */
async function fetchBytes(url: string): Promise<ArrayBuffer> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`fetch ${url}: HTTP ${res.status}`);
  return res.arrayBuffer();
}

export class RutLsp {
  private e: LspExports;
  private enc = new TextEncoder();
  private dec = new TextDecoder();
  private legendNames: string[];

  private constructor(inst: WebAssembly.Instance) {
    this.e = inst.exports as unknown as LspExports;
    // the artifact and the page ship together — an export surface that
    // doesn't match THIS contract is an invalid artifact (loud, boot
    // turns it into the error panel, never a half-working page)
    for (const n of [
      "rut_begin",
      "rut_alloc",
      "rut_legend",
      "rut_analyze",
    ] as const) {
      if (typeof this.e[n] !== "function") {
        throw new Error(`invalid rut-lsp.wasm: missing export ${n}`);
      }
    }
    this.legendNames = this.call(() => this.e.rut_legend());
  }

  /** Load the module once. `load` overrides the byte source (the
   * headless smoke reads the same file from disk); everything after the
   * bytes — instantiate, export validation, the legend read — is the
   * one shared path the browser uses. Throws on any failure; the App
   * maps that onto the phase-1 error-panel shape. */
  static async boot(load?: (url: string) => Promise<ArrayBuffer>): Promise<RutLsp> {
    const bytes = await (load ?? fetchBytes)("rut-lsp.wasm");
    const { instance } = await WebAssembly.instantiate(bytes, {});
    return new RutLsp(instance);
  }

  /** token-type names, in legend index order (the CSS map keys on
   * these names, never on a hardcoded order) */
  get legend(): string[] {
    return this.legendNames;
  }

  /** open / FULL-sync change: store the doc, get diags + tokens in the
   * one response (the survey's call) */
  analyze(uri: string, src: string): LspAnalysis {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      const [s, sl] = this.put(src);
      return this.e.rut_analyze(u, ul, s, sl);
    });
  }

  // ---- the raw protocol (the extension's wasm.ts twin) ----

  /// every (ptr, len) pair lands in the per-request arena; results are
  /// read before the next request resets it
  private put(s: string): [number, number] {
    const bytes = this.enc.encode(s);
    const ptr = this.e.rut_alloc(bytes.length);
    if (ptr === 0) throw new Error("rut-lsp wasm: arena overflow");
    new Uint8Array(this.e.memory.buffer, ptr, bytes.length).set(bytes);
    return [ptr, bytes.length];
  }

  /// `rut_begin` -> write inputs -> call -> read the JSON envelope
  private call<T>(write: () => number): T {
    this.e.rut_begin();
    const ptr = write();
    if (ptr === 0) throw new Error("rut-lsp wasm: arena overflow");
    const len = new DataView(this.e.memory.buffer, ptr, 4).getUint32(0, true);
    const json = this.dec.decode(new Uint8Array(this.e.memory.buffer, ptr + 4, len));
    return JSON.parse(json) as T;
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
