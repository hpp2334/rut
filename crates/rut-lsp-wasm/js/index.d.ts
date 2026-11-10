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
 * workspace origin, a store-mounted dep file, or (the core prelude)
 * a repo-relative source path the host resolves against the
 * workspace.
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
 * the injected, READ-ONLY host environment — the capability the core's
 * dep walk reads (phase: dep-walk-into-crate). Paths are rooted
 * strings; the crate joins them with `/` and the host normalizes. The
 * grant is read-only by construction: no write, no delete, no fetch
 * exists on this interface.
 *
 * The one machinery addition is the OPTIONAL clock: `wakeAfter` arms
 * the core's debounce window (plain `setTimeout` covers it) and hands
 * back a `wake` that posts one identity-free tick into the core's own
 * event loop — the host supplies power, the ACTOR owns every when.
 * Absent means "no clock": every deferral degrades to eager, in-turn
 * work. No business callback exists on this interface.
 */
export interface HostEnv {
    /** one file's bytes — `null` on a miss, never a throw */
    read(path: string): Uint8Array | null;
    /** one directory's entry names — `null` when there is no dir */
    readDir(path: string): string[] | null;
    /** does the path name a directory? */
    isDir(path: string): boolean;
    /** the hint lane — a message for the user, error-flagged or not */
    notify(message: string, isError: boolean): void;
    /**
     * the clock — schedule `wake`'s call `ms` milliseconds out (plain
     * `setTimeout` covers it). Machinery only: the core's deferred
     * work arms it; the host never interprets a wake.
     */
    wakeAfter?(ms: number, wake: () => void): void;
}

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
  consts: Array<[string, number]>;
  deps: DepRow[];
  dev_deps: DepRow[];
}

/** a stamped file's own `mod` declarations — the mount walk's next hop */
export interface ModDecls {
  decls: Array<{ name: string; vis: 'pub' | 'mod' }>;
}

/**
 * one url dep the read-only walk refused to mount — the informational
 * hint row a host surfaces (the `rut install` recipe itself already
 * rode the env's notify lane, once per reason). `sha256` is `null`
 * for an unpinned row: the honest refusal, never a guessed pin.
 */
export interface MissingDep {
  url: string;
  sha256: string | null;
  cache_path: string;
}

/**
 * `resolveDeps`'s answer — the walk's refused url rows. Mounted rows
 * carry no row: the defs store they mounted into IS the answer,
 * exactly the world an `addDef`-indexed file leaves.
 */
export interface WalkOut {
  missing: MissingDep[];
}

// ---- the binding ----

/**
 * the language core as the host holds it — one per editing session.
 * Every method is an async RPC bridge over the core's event loop (the
 * actor seam): hosts await everything, and the loop's FIFO keeps
 * concurrent calls resolving in send order.
 */
export class RutLsp {
  /** use {@link RutLsp.load} — the core needs an instantiated wasm */
  private constructor();

  /**
   * Load the shipped artifact once (the glue's `init` is module-global).
   * The core prelude is already indexed by the constructor. `env` — the
   * injected read-only host environment (read/readDir/isDir/notify,
   * plus the optional clock `wakeAfter`) — is optional: when present
   * it rides `set_env` before the handle returns (the dep walk's
   * capability, and the clock its deferred walks arm); when omitted
   * the core stays env-less, exactly the shape every host boots today.
   */
  static load(bytes: Uint8Array | ArrayBuffer, env?: HostEnv): Promise<RutLsp>;

  /** the semantic-token legend, names in token-type index order */
  legend(): Promise<string[]>;

  /** open / FULL-sync change: store the doc, answer diags + tokens + symbols */
  analyze(uri: string, src: string): Promise<Analysis>;

  /** close: drop the doc (the workspace/std index stays) */
  forget(uri: string): Promise<void>;

  /** hover at an LSP position — `null` when there is nothing to say */
  hover(uri: string, line: number, character: number): Promise<LspHover | null>;

  /**
   * completions at an LSP position (`[]` when nothing resolves). Pass
   * `src` — the document's CURRENT text — to store it first (the
   * doc-sync lane: completion fires on the keystroke, an analyze
   * debounce may still be pending).
   */
  complete(uri: string, line: number, character: number, src: string): Promise<LspCompletionItem[]>;

  /** go-to-definition at an LSP position — `[]` when nothing resolves */
  definition(uri: string, line: number, character: number): Promise<LspLocation[]>;

  /** go-to-type-definition at an LSP position — `[]` when nothing resolves */
  typeDefinition(uri: string, line: number, character: number): Promise<LspLocation[]>;

  /** inlay hints for a document range — `[]` when nothing resolves */
  inlayHint(uri: string, start: LspPosition, end: LspPosition): Promise<LspInlayHint[]>;

  /** find references at an LSP position — `[]` when nothing resolves */
  references(
    uri: string,
    line: number,
    character: number,
    includeDeclaration: boolean
  ): Promise<LspLocation[]>;

  /** signature help at an LSP position — `null` when there is none */
  signatureHelp(uri: string, line: number, character: number): Promise<LspSignatureHelp | null>;

  /** index one workspace file (the host's stand-in for the fs walk) */
  addDef(uri: string, src: string): Promise<void>;

  /** parse one `rut.jsonc` — the dep table, or the error envelope */
  parseManifest(text: string): Promise<DepTable | Envelope>;

  /** index one source file under its module NAME at a MOD PATH (the scan lane) */
  addDefMod(uri: string, name: string, modPath: string, src: string): Promise<ModDecls | Envelope>;

  /**
   * re-inject the read-only host environment (replace-on-reinject —
   * the same lane `load`'s optional `env` rides, for hosts that swap
   * the capability after boot)
   */
  setEnv(env: HostEnv): Promise<void>;

  /**
   * the READ-ONLY dep walk: one call, the whole walked world — each
   * root's `rut.lock` feeds the pin ledger, its `rut.jsonc` walks
   * (entries, mod trees, path rows recursing, url rows mounting from
   * the global dep store when its meta vouches the pin). The answer
   * carries the refused url rows (`missing`); the mounted world lands
   * in the core's defs store, exactly like `addDef`-indexed files.
   */
  resolveDeps(roots: string[]): Promise<WalkOut>;

  /**
   * stir the dep walk — the level-triggered door onto the same walk
   * `resolveDeps` runs eagerly: the walk defers under the core's own
   * debounce window (200ms, armed through the env's optional
   * `wakeAfter` clock) and coalesces, so a burst of stirs walks once
   * over the freshest roots. This promise resolves with the `WalkOut`
   * once the deferred walk runs — the answer rides this call's own
   * reply channel, no push lane exists. A stir superseded by a later
   * one resolves `null` (merged: the LAST stir's promise carries the
   * world). No clock on the env ⇒ the walk runs eagerly in-turn,
   * exactly `resolveDeps`.
   */
  stir(roots: string[]): Promise<WalkOut | null>;
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

// ---- the dep engine shell ----

/** the dep engine's construction options */
export interface DepEngineOptions {
  /**
   * the read-only host environment — re-injected before each run
   * (replace-on-reinject). Optional: a core that carried one at
   * `load` keeps it.
   */
  env?: HostEnv;
  /** the workspace roots — a host callback, re-read at every run */
  roots: () => string[];
  /** the coalescing window in milliseconds (default 200) */
  debounceMs?: number;
}

/** the dep engine handle — stir to schedule, dispose to retire */
export interface DepEngine {
  /** arm (or re-arm) the debounce; a stir mid-run coalesces into ONE rerun */
  stir(): void;
  /** retire the engine — pending timers clear, later stirs are no-ops */
  dispose(): void;
}

/**
 * The dep engine shell: debounce + one-run-in-flight +
 * rerun-once-if-stirred over `resolveDeps` (the extension's walk
 * scheduler law, moved into the crate face — `setTimeout` + `Promise`
 * only, environment-neutral).
 */
export function depEngine(rut: RutLsp, opts: DepEngineOptions): DepEngine;
