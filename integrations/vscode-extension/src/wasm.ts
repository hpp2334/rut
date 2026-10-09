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
  /// the auto-import tier sorts after every local item (`~` prefix)
  sortText?: string;
  /// the auto-import insert (`use mod::Name;`) — a fresh line after the
  /// last leading `use` (or at the body top)
  additionalTextEdits?: Array<{ range: LspRange; newText: string }>;
}

/// an LSP InlayHint — the server supplies position + label (with the
/// colon) + kind; the client renders the dotted decoration
export interface LspInlayHint {
  position: LspPosition;
  label: string;
  kind?: number;
  tooltip?: { kind: string; value: string } | string;
}

/// an LSP ParameterInformation — the label is the param as written (a
/// substring of the signature label)
export interface LspParameterInformation {
  label: string;
}

/// an LSP SignatureInformation — the verbatim `fn` signature plus its
/// parameters as written
export interface LspSignatureInformation {
  label: string;
  documentation?: { kind: string; value: string };
  parameters?: LspParameterInformation[];
}

/// an LSP SignatureHelp — one signature, the active slot computed from
/// the comma/paren depth at the request position
export interface LspSignatureHelp {
  signatures: LspSignatureInformation[];
  activeSignature?: number;
  activeParameter?: number;
}

// ---- the deps face (rut_parse_manifest / rut_add_def_named /
// rut_add_bundle) — the envelope spellings mirror the Rust side's own
// field names (wire truth, `null` for the absent Options) ----

/// where a dep row's bytes live: a directory beside the manifest, or a
/// remote `.rutbundle` with its sha256 pin and the Rust-computed cache
/// path (`.rut/cache/<sha256(url)>.rutbundle` — JS never hashes URLs)
export type DepSource =
  | { kind: 'path'; dir: string }
  | { kind: 'url'; url: string; sha256: string | null; cache_path: string };

/// one dep row (`deps` / `dev-deps` / `peer-deps`); `optional`/`lib`
/// are the peer descriptor's bits
export interface DepRow {
  name: string;
  source: DepSource;
  optional: boolean;
  lib: string | null;
}

/// one parsed manifest, editor-shaped: the module's identity (name,
/// entry sources with the mode each indexes in, namespace, consts)
/// plus its dep rows
export interface DepTable {
  name: string | null;
  entries: Array<{ path: string; mode: 'Decl' | 'Impl' }>;
  namespace: string | null;
  consts: Array<[string, number]>;
  deps: DepRow[];
  dev_deps: DepRow[];
  peer_deps: DepRow[];
}

/// `rut_parse_manifest` / `rut_add_bundle` answer the error envelope on
/// bad bytes — the host hints, never silent unparsed state
export type Envelope = { error: string };

export function isEnvelope(e: unknown): e is Envelope {
  return typeof (e as Envelope).error === 'string';
}

/// `rut_add_def_mod`'s success envelope — the indexed file's own `mod`
/// declarations, the mount walk's next hops (name + edge vis)
export interface ModDecls {
  decls: Array<{ name: string; vis: 'pub' | 'mod' }>;
}

/// `rut_add_bundle`'s success envelope — the module + entry paths indexed
export interface BundleIndexed {
  module: string;
  namespace: string | null;
  consts: Array<[string, number]>;
  files: string[];
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
  rut_inlay(
    uriPtr: number,
    uriLen: number,
    startLine: number,
    startCh: number,
    endLine: number,
    endCh: number
  ): number;
  rut_references(uriPtr: number, uriLen: number, line: number, ch: number, includeDecl: number): number;
  rut_signature_help(uriPtr: number, uriLen: number, line: number, ch: number): number;
  rut_add_def(uriPtr: number, uriLen: number, srcPtr: number, srcLen: number): void;
  rut_parse_manifest(ptr: number, len: number): number;
  rut_add_def_named(
    uriPtr: number,
    uriLen: number,
    namePtr: number,
    nameLen: number,
    srcPtr: number,
    srcLen: number
  ): void;
  rut_add_def_mod(
    uriPtr: number,
    uriLen: number,
    namePtr: number,
    nameLen: number,
    modPathPtr: number,
    modPathLen: number,
    srcPtr: number,
    srcLen: number
  ): number;
  rut_add_bundle(ptr: number, len: number): number;
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

  /// inlay hints for a document range — the inline inference display
  /// ([] when nothing resolves)
  inlayHint(uri: string, start: LspPosition, end: LspPosition): LspInlayHint[] {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      return this.e.rut_inlay(u, ul, start.line, start.character, end.line, end.character);
    });
  }

  /// find references at an LSP position — the definition index read
  /// backwards ([] when nothing resolves); includeDeclaration follows
  /// the LSP toggle
  references(uri: string, line: number, character: number, includeDeclaration: boolean): LspLocation[] {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      return this.e.rut_references(u, ul, line, character, includeDeclaration ? 1 : 0);
    });
  }

  /// signature help at an LSP position — null when there is no call to
  /// help with or the callee does not resolve to one known signature
  signatureHelp(uri: string, line: number, character: number): LspSignatureHelp | null {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      return this.e.rut_signature_help(u, ul, line, character);
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

  /// parse one `rut.jsonc` — the dep-table JSON out, or the error
  /// envelope (the host surfaces it as a hint). The wasm parses; JS
  /// moves bytes.
  parseManifest(text: string): DepTable | Envelope {
    return this.call(() => {
      const [p, l] = this.put(text);
      return this.e.rut_parse_manifest(p, l);
    });
  }

  /// index one dep source file under its module NAME (the dep walk's
  /// per-file move) — `uri` is the file's real on-disk URI so F12
  /// jumps land. The mode rides the URI convention (`.d.rut` → Decl),
  /// same as `addDef`.
  addDefNamed(uri: string, name: string, src: string): void {
    this.exec(() => {
      const [u, ul] = this.put(uri);
      const [n, nl] = this.put(name);
      const [s, sl] = this.put(src);
      this.e.rut_add_def_named(u, ul, n, nl, s, sl);
    });
  }

  /// index one dep source file under its module NAME at a MOD PATH —
  /// the mod-aware twin of `addDefNamed` (the dep walk's per-file move
  /// for a mod-carrying package: the root at `''`, each child at its
  /// path). Returns the parsed file's own `mod` declarations — the
  /// host's mount walk reads its next hop from them (the module
  /// parses; JS moves bytes).
  addDefMod(uri: string, name: string, modPath: string, src: string): ModDecls | Envelope {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      const [n, nl] = this.put(name);
      const [p, pl] = this.put(modPath);
      const [s, sl] = this.put(src);
      return this.e.rut_add_def_mod(u, ul, n, nl, p, pl, s, sl);
    });
  }

  /// index one `.rutbundle` (the archive's own manifest names the
  /// module; its entry sources index under it) — the indexed envelope,
  /// or the error one
  addBundle(bytes: Uint8Array): BundleIndexed | Envelope {
    return this.call(() => {
      const [p, l] = this.putBytes(bytes);
      return this.e.rut_add_bundle(p, l);
    });
  }

  // ---- the raw protocol ----

  /// every (ptr, len) pair lands in the per-request arena; results are
  /// read before the next request resets it
  private put(s: string): [number, number] {
    return this.putBytes(this.enc.encode(s));
  }

  private putBytes(bytes: Uint8Array): [number, number] {
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
