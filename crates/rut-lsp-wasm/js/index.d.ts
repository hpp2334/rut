// The rut-lsp-wasm ABI — the ONE declaration file for the JS face.
//
// `index.js` parses the wasm-bindgen glue's JSON answers; this file
// names their shapes. Hosts (the VS Code extension, the demo
// playground) import both from here, so there is no second copy of
// these interfaces in any host.
//
// Two locks keep it true, because a `.d.ts` alone cannot:
//   1. `tsc --noEmit` in every consumer — a host that spells a field
//      wrong stops compiling;
//   2. `integrations/vscode-extension/test/e2e-wasm.js` drives the
//      SHIPPED artifact and asserts each envelope's exact key set —
//      the runtime half, which catches a Rust-side rename that no type
//      checker can see.
//
// Envelope payloads that are `ls_types` values (diagnostics, symbols,
// hover, completion items, inlay hints, signature help) are declared
// here as the subset the hosts consume; the wasm may carry more.

// ---- LSP values ----

export interface LspPosition {
  line: number;
  character: number;
}

export interface LspRange {
  start: LspPosition;
  end: LspPosition;
}

/** an LSP diagnostic — `severity` is 1..4 (Error..Hint), `source` is `rut` */
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

export interface LspSymbol {
  name: string;
  detail?: string;
  /** LSP SymbolKind (1-based: File = 1, …) */
  kind: number;
  range: LspRange;
  selectionRange: LspRange;
  children?: LspSymbol[];
}

export interface LspHover {
  contents: { kind: string; value: string };
  range?: LspRange;
}

/**
 * an LSP Location — a definition target. `uri` is the doc itself, a
 * workspace origin, or (the embedded std surface) a repo-relative
 * source path the host resolves against the workspace.
 */
export interface LspLocation {
  uri: string;
  range: LspRange;
}

export interface LspCompletionItem {
  label: string;
  /** LSP CompletionItemKind (1-based) */
  kind?: number;
  detail?: string;
  documentation?: { kind: string; value: string };
  /** the auto-import tier sorts after every local item (`~` prefix) */
  sortText?: string;
  /** the auto-import insert (`use mod::Name;`) — a fresh line */
  additionalTextEdits?: Array<{ range: LspRange; newText: string }>;
}

export interface LspInlayHint {
  position: LspPosition;
  label: string;
  /** 1 = Type, 2 = Parameter (the LSP and vscode numbering agree) */
  kind?: number;
  tooltip?: { kind: string; value: string } | string;
}

export interface LspParameterInformation {
  label: string;
}

export interface LspSignatureInformation {
  label: string;
  documentation?: { kind: string; value: string };
  parameters?: LspParameterInformation[];
  /** the active slot as the per-signature spelling (LSP 3.16 — the
   * envelope may carry both this and `activeParameter` on the help) */
  activeParameter?: number;
}

export interface LspSignatureHelp {
  signatures: LspSignatureInformation[];
  activeSignature?: number;
  activeParameter?: number;
}

// ---- the answers ----

/** `analyze`'s answer: merged diags + the flat token stream + symbols */
export interface Analysis {
  diags: LspDiag[];
  /** `data` is the flat LSP wire array (5 u32s per token) */
  tokens: { data: number[] };
  symbols: LspSymbol[];
}

/** a parse answer that is the error envelope, never the asked shape */
export interface Envelope {
  error: string;
}

export function isEnvelope(v: unknown): v is Envelope;

// ---- the deps face ----

/**
 * where a dep row's bytes live: a directory beside the manifest, or a
 * remote archive with its lock-sourced sha256 pin and the
 * Rust-computed cache path (JS never hashes URLs)
 */
export type DepSource =
  | { kind: 'path'; dir: string }
  | { kind: 'url'; url: string; sha256: string | null; cache_path: string };

/** one dep row (`deps` / `dev-deps`) */
export interface DepRow {
  name: string;
  source: DepSource;
  optional: boolean;
  lib: string | null;
}

/** the parsed `rut.jsonc` */
export interface DepTable {
  name: string | null;
  entries: Array<{ path: string; mode: 'Decl' | 'Impl' }>;
  namespace: string | null;
  consts: Array<[string, number]>;
  deps: DepRow[];
  dev_deps: DepRow[];
}

/** a stamped file's own `mod` declarations — the mount walk's next hop */
export interface ModDecls {
  decls: Array<{ name: string; vis: 'pub' | 'mod' }>;
}

/** `addBundle`'s success envelope */
export interface BundleIndexed {
  module: string;
  namespace: string | null;
  consts: Array<[string, number]>;
  files: string[];
}

/** one extracted file — the unpack WITHOUT indexing */
export interface BundleExtractedFile {
  path: string;
  src: string;
  mode: 'Decl' | 'Impl';
  mod_path: string | null;
}

/** `extractBundle`'s success envelope */
export interface BundleExtracted {
  module: string;
  namespace: string | null;
  consts: Array<[string, number]>;
  files: BundleExtractedFile[];
}

/**
 * `extractSrczip`'s success envelope — the source zip's own package
 * files plus the unpack law's ingredients: the archive's manifest text
 * and the embedded `deps/<identity>/` groups
 */
export interface SrczipExtracted extends BundleExtracted {
  manifest: string;
  embedded: Array<{ path: string; src: string }>;
}

// ---- the binding ----

/** the language core as the host holds it — one per editing session */
export class RutLsp {
  /** use {@link RutLsp.load} — the core needs an instantiated wasm */
  private constructor();

  /**
   * Load the shipped artifact once (the glue's `init` is module-global).
   * The std surface is already indexed by the constructor.
   */
  static load(bytes: Uint8Array | ArrayBuffer): Promise<RutLsp>;

  /** the semantic-token legend, names in token-type index order */
  legend(): string[];

  /** open / FULL-sync change: store the doc, answer diags + tokens + symbols */
  analyze(uri: string, src: string): Analysis;

  /** close: drop the doc (the workspace/std index stays) */
  forget(uri: string): void;

  /** hover at an LSP position — `null` when there is nothing to say */
  hover(uri: string, line: number, character: number): LspHover | null;

  /**
   * completions at an LSP position (`[]` when nothing resolves). Pass
   * `src` — the document's CURRENT text — to store it first (the
   * doc-sync lane: completion fires on the keystroke, an analyze
   * debounce may still be pending).
   */
  complete(uri: string, line: number, character: number, src?: string): LspCompletionItem[];

  /** go-to-definition at an LSP position — `[]` when nothing resolves */
  definition(uri: string, line: number, character: number): LspLocation[];

  /** go-to-type-definition at an LSP position — `[]` when nothing resolves */
  typeDefinition(uri: string, line: number, character: number): LspLocation[];

  /** inlay hints for a document range — `[]` when nothing resolves */
  inlayHint(uri: string, start: LspPosition, end: LspPosition): LspInlayHint[];

  /** find references at an LSP position — `[]` when nothing resolves */
  references(
    uri: string,
    line: number,
    character: number,
    includeDeclaration: boolean
  ): LspLocation[];

  /** signature help at an LSP position — `null` when there is none */
  signatureHelp(uri: string, line: number, character: number): LspSignatureHelp | null;

  /** index one workspace file (the host's stand-in for the fs walk) */
  addDef(uri: string, src: string): void;

  /** parse one `rut.jsonc` — the dep table, or the error envelope */
  parseManifest(text: string): DepTable | Envelope;

  /** hand the core the workspace root's `rut.lock` — the pin ledger */
  setLock(text: string): { pins: number } | Envelope;

  /** hand the core the direct-dep module names (the auto-import gate) */
  setDirectDeps(names: string[]): { direct: number } | Envelope;

  /** index one dep source file under its module NAME */
  addDefNamed(uri: string, name: string, src: string): void;

  /** index one dep source file under its module NAME at a MOD PATH */
  addDefMod(uri: string, name: string, modPath: string, src: string): ModDecls | Envelope;

  /** index one `.rutbundle` (the archive's own manifest names the module) */
  addBundle(bytes: Uint8Array): BundleIndexed | Envelope;

  /** unpack one `.rutbundle` WITHOUT indexing — the host owns the disk */
  extractBundle(bytes: Uint8Array): BundleExtracted | Envelope;

  /** unpack one SOURCE ZIP without indexing — the source transport's twin */
  extractSrczip(bytes: Uint8Array): SrczipExtracted | Envelope;
}

/** one decoded semantic token — absolute 0-based LSP coordinates */
export interface DecodedToken {
  line: number;
  start: number;
  length: number;
  /** index into `legend()` — never a hardcoded position */
  type: number;
}

/** decode the flat `SemanticTokens.data` wire array */
export function decodeTokens(data: number[]): DecodedToken[];
