// The rut-lsp-wasm raw ABI (RFC 0041 §2) — the same envelope protocol
// `rut-wasm` uses: inputs are written into the module's arena via
// `rut_alloc`, results come back as `[u32 little-endian length][bytes]`
// JSON at the returned pointer. Every request starts with `rut_begin()`
// (the arena is per-request — an LSP session is long-lived) and its
// envelope is read before the next request. Single-threaded JS makes
// that trivially safe.

export interface LspPosition {
  line: number;
  character: number;
}

export interface LspRange {
  start: LspPosition;
  end: LspPosition;
}

export interface LspDiag {
  range: LspRange;
  severity?: number;
  source?: string;
  message: string;
  relatedInformation?: Array<{
    location: { uri: string; range: LspRange };
    message: string;
  }>;
}

// the flat `SemanticTokens.data` wire array (5 u32s per token)
export interface Analysis {
  diags: LspDiag[];
  tokens: { data: number[] };
  symbols: LspSymbol[];
}

export interface LspSymbol {
  name: string;
  detail?: string;
  kind: number;
  range: LspRange;
  selectionRange: LspRange;
  children?: LspSymbol[];
}

export interface LspHover {
  contents: { kind: string; value: string };
  range?: LspRange;
}

/// an LSP Location — a definition target. `uri` is the doc itself, a
/// workspace origin, or (embedded std surface) a repo-relative source
/// path the extension resolves against the workspace
export interface LspLocation {
  uri: string;
  range: LspRange;
}

export interface LspCompletionItem {
  label: string;
  kind?: number;
  detail?: string;
  documentation?: { kind: string; value: string };
}

interface RutExports {
  memory: WebAssembly.Memory;
  rut_begin(): void;
  rut_alloc(len: number): number;
  rut_legend(): number;
  rut_analyze(uriPtr: number, uriLen: number, srcPtr: number, srcLen: number): number;
  rut_forget(uriPtr: number, uriLen: number): void;
  rut_hover(uriPtr: number, uriLen: number, line: number, ch: number): number;
  rut_complete(uriPtr: number, uriLen: number, line: number, ch: number): number;
  rut_definition(uriPtr: number, uriLen: number, line: number, ch: number): number;
  rut_type_definition(uriPtr: number, uriLen: number, line: number, ch: number): number;
  rut_add_def(uriPtr: number, uriLen: number, srcPtr: number, srcLen: number): void;
}

/// everything the module exports (the ABI surface + its linear memory)
type WasmExports = RutExports;

export class RutWasm {
  private constructor(private e: WasmExports) {}

  static async load(wasmPath: string): Promise<RutWasm> {
    const { readFile } = await import('node:fs/promises');
    const bytes = await readFile(wasmPath);
    const { instance } = await WebAssembly.instantiate<WasmExports>(bytes, {});
    return new RutWasm(instance.exports);
  }

  /// the semantic-token legend, names in token-type index order
  legend(): string[] {
    return this.call(() => this.e.rut_legend());
  }

  /// open / FULL-sync change: store the doc, get diags + tokens + symbols
  analyze(uri: string, src: string): Analysis {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      const [s, sl] = this.put(src);
      return this.e.rut_analyze(u, ul, s, sl);
    });
  }

  /// close: drop the doc (the workspace/std index stays)
  forget(uri: string): void {
    this.exec(() => {
      const [u, ul] = this.put(uri);
      this.e.rut_forget(u, ul);
    });
  }

  hover(uri: string, line: number, character: number): LspHover | null {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      return this.e.rut_hover(u, ul, line, character);
    });
  }

  complete(uri: string, line: number, character: number): LspCompletionItem[] {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      return this.e.rut_complete(u, ul, line, character);
    });
  }

  /// go-to-definition at an LSP position — [] when nothing resolves
  definition(uri: string, line: number, character: number): LspLocation[] {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      return this.e.rut_definition(u, ul, line, character);
    });
  }

  /// go-to-type-definition at an LSP position — [] when nothing resolves
  typeDefinition(uri: string, line: number, character: number): LspLocation[] {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      return this.e.rut_type_definition(u, ul, line, character);
    });
  }

  /// index one workspace file (the host's stand-in for the native
  /// server's fs walk; the 500-file cap is the caller's)
  addDef(uri: string, src: string): void {
    this.exec(() => {
      const [u, ul] = this.put(uri);
      const [s, sl] = this.put(src);
      this.e.rut_add_def(u, ul, s, sl);
    });
  }

  // ---- the raw protocol ----

  /// every (ptr, len) pair lands in the per-request arena; results are
  /// read before the next request resets it
  private put(s: string): [number, number] {
    const bytes = this.enc.encode(s);
    const ptr = this.e.rut_alloc(bytes.length);
    if (ptr === 0) {
      throw new Error('rut wasm: arena overflow');
    }
    new Uint8Array(this.e.memory.buffer, ptr, bytes.length).set(bytes);
    return [ptr, bytes.length];
  }

  /// `rut_begin` -> write inputs -> call -> read the JSON envelope
  private call<T>(write: () => number): T {
    this.e.rut_begin();
    const ptr = write();
    if (ptr === 0) {
      throw new Error('rut wasm: arena overflow');
    }
    const len = new DataView(this.e.memory.buffer, ptr, 4).getUint32(0, true);
    const json = this.dec.decode(new Uint8Array(this.e.memory.buffer, ptr + 4, len));
    return JSON.parse(json) as T;
  }

  /// `rut_begin` -> write inputs -> call (no envelope: void exports)
  private exec(write: () => void): void {
    this.e.rut_begin();
    write();
  }

  private enc = new TextEncoder();
  private dec = new TextDecoder();
}
