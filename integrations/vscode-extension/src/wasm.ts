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

/// one dep row (`deps` / `dev-deps`); `optional`/`lib`
/// are inert descriptor bits the wire carries
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

/// one unpacked bundle file — the archive path as written, its mode,
/// and the index lane's mod path (`''` the root `mod.rut`, `<path>` a
/// mounted child, `null` a flat file)
export interface BundleExtractedFile {
  path: string;
  src: string;
  mode: 'Decl' | 'Impl';
  mod_path: string | null;
}

/// `rut_extract_bundle`'s success envelope — the module + its editable
/// surface, extracted WITHOUT indexing (the host owns the disk: it
/// writes the files and re-indexes them through `addDefMod` /
/// `addDefNamed`, real `file://` origins)
export interface BundleExtracted {
  module: string;
  namespace: string | null;
  consts: Array<[string, number]>;
  files: BundleExtractedFile[];
}

/// `rut_extract_srczip`'s success envelope — the same shape, over a
/// source zip's own package files (`src/**` at zip-relative paths)
export type SrczipExtracted = BundleExtracted;

interface RutExports {
  memory: WebAssembly.Memory;
  rut_begin(): void;
  rut_alloc(len: number): number;
  rut_legend(): number;
  rut_analyze(uriPtr: number, uriLen: number, srcPtr: number, srcLen: number): number;
  rut_forget(uriPtr: number, uriLen: number): void;
  rut_hover(uriPtr: number, uriLen: number, line: number, ch: number): number;
  rut_complete(uriPtr: number, uriLen: number, line: number, ch: number): number;
  /// the doc-sync completion entry: the CURRENT text is stored before
  /// the tiers run (completion fires on the keystroke, analyze debounces)
  rut_complete_in(uriPtr: number, uriLen: number, srcPtr: number, srcLen: number, line: number, ch: number): number;
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
  /// the url→pin ledger the NEXT `rut_parse_manifest` stamps onto its
  /// url rows — the workspace root's `rut.lock` bytes
  rut_set_lock(ptr: number, len: number): number;
  /// the direct-dep names (JSON array) — the stamp the dep indexes
  /// carry for the auto-import tier's gate
  rut_set_direct_deps(ptr: number, len: number): number;
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
  rut_extract_bundle(ptr: number, len: number): number;
  rut_extract_srczip(ptr: number, len: number): number;
}

/// everything the module exports (the ABI surface + its linear memory)
type WasmExports = RutExports;

/// the synthetic error envelope an OLDER artifact's missing extract
/// export answers with — the host surfaces it as the one hint, and the
/// dep walk survives a stale install instead of dying on a TypeError
const STALE_ARTIFACT =
  'the loaded rut-lsp.wasm predates this dep transport — rebuild and reinstall the ' +
  'extension (`npm run build:wasm`, `npm run package`, install the vsix), then reload the window';

export class RutWasm {
  /// feature probe — exports an older artifact may lack, checked once at
  /// load (`complete` probes `rut_complete_in` per call, the same
  /// backward-compat law). `rut_extract_srczip` is the source-zip dep
  /// transport's extract; a same-version-but-older install (the version
  /// number outlived the lane) would otherwise crash the dep walk the
  /// moment a `.zip` cache entry mounts.
  readonly hasSrczip: boolean;
  readonly hasBundleExtract: boolean;

  private constructor(private e: WasmExports) {
    this.hasSrczip = typeof e.rut_extract_srczip === 'function';
    this.hasBundleExtract = typeof e.rut_extract_bundle === 'function';
  }

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

  /// completions at an LSP position. `src` — the document's CURRENT
  /// text — rides the doc-sync entry (`rut_complete_in`): the bytes are
  /// stored before the tiers run, so the `.` the user just typed is in
  /// the text they lex even while the analyze debounce is pending.
  /// Without `src`, or against an older artifact without the entry, the
  /// stored read (`rut_complete`) answers — backward compatible both ways.
  complete(uri: string, line: number, character: number, src?: string): LspCompletionItem[] {
    return this.call(() => {
      const [u, ul] = this.put(uri);
      if (src !== undefined && typeof this.e.rut_complete_in === 'function') {
        const [s, sl] = this.put(src);
        return this.e.rut_complete_in(u, ul, s, sl, line, character);
      }
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

  /// hand the wasm the workspace root's `rut.lock` bytes — the url→pin
  /// ledger the NEXT `parseManifest` stamps onto its url rows (the lock
  /// is the pin's only source; the manifest grammar retired `sha256`).
  /// `{"pins": N}`, or the error envelope for a malformed lock (the
  /// empty ledger stands — url rows answer `sha256: null`, the honest
  /// unpinned refusal, never guessed pins).
  setLock(text: string): { pins: number } | Envelope {
    return this.call(() => {
      const [p, l] = this.put(text);
      return this.e.rut_set_lock(p, l);
    });
  }

  /// hand the wasm the direct-dep module names — the open document's
  /// own package name plus every dep row its ROOT manifest declares.
  /// Dep indexes named here stamp the direct-dep marker the
  /// auto-import tier gates on (a transitive-only dep's `use` is the
  /// compiler's D5 error), so the host calls this when it parses a
  /// root manifest, BEFORE mounting its rows. Names accumulate in the
  /// wasm's set (multi-folder walks, re-walks).
  setDirectDeps(names: string[]): { direct: number } | Envelope {
    return this.call(() => {
      const [p, l] = this.put(JSON.stringify(names));
      return this.e.rut_set_direct_deps(p, l);
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

  /// unpack one `.rutbundle` WITHOUT indexing — the pure-extract twin
  /// of `addBundle`: bytes in, the editable surface out. The host
  /// writes the files and re-indexes them through `addDefMod` /
  /// `addDefNamed` (real `file://` origins — the go-to-definition
  /// face), so this lane has no index side effect. The error envelope
  /// on a corrupt/foreign archive.
  extractBundle(bytes: Uint8Array): BundleExtracted | Envelope {
    if (!this.hasBundleExtract) {
      return { error: STALE_ARTIFACT };
    }
    return this.call(() => {
      const [p, l] = this.putBytes(bytes);
      return this.e.rut_extract_bundle(p, l);
    });
  }

  /// unpack one SOURCE ZIP without indexing — the source transport's
  /// extract twin: the zip's own `src/**` files out, mod paths derived
  /// from the `src/` shell. The error envelope on a corrupt/foreign
  /// archive.
  extractSrczip(bytes: Uint8Array): SrczipExtracted | Envelope {
    if (!this.hasSrczip) {
      return { error: STALE_ARTIFACT };
    }
    return this.call(() => {
      const [p, l] = this.putBytes(bytes);
      return this.e.rut_extract_srczip(p, l);
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
