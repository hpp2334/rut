// rut-lsp-wasm's JS face — the typed binding every editor consumes.
//
// The wasm-bindgen glue (`../pkg/rut_lsp_wasm.js`, gitignored build
// output) spells every answer as a JSON string; this module is the ONE
// place that parses them and gives the shapes names. Hosts never import
// the glue directly: the VS Code extension and the demo playground both
// import THIS file, so the ABI has one runtime face and one declaration
// file (`index.d.ts`) instead of one copy per host.
//
// The environment-neutral law: no `node:*`, no `fetch`, no DOM. `load`
// takes the artifact BYTES; who produced them (Node's fs, a browser
// fetch, an in-process cache) is the host's business. That is what lets
// the same file serve a Node extension and a browser playground.
//
// Single-threaded JS, one RefCell borrow per call: the instance owns its
// session state (open docs + the index) for its lifetime — one `RutLsp`
// per editing session, mutated per request.

import init, { RutLsp as Core } from '../pkg/rut_lsp_wasm.js';

/** the language core as the host holds it — one per editing session */
export class RutLsp {
  /** @param {Core} core the wasm-bindgen instance (see {@link RutLsp.load}) */
  constructor(core) {
    this.core = core;
  }

  /**
   * Load the module once: the shipped artifact's bytes in, a live core
   * out (the std surface is already indexed by the constructor).
   * The glue's `init` is module-global — one `load` per process; a
   * second call reuses the instantiated module.
   *
   * @param {Uint8Array | ArrayBuffer} bytes the `rut-lsp.wasm` artifact
   * @returns {Promise<RutLsp>}
   */
  static async load(bytes) {
    await init({
      module_or_path: bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes),
    });
    return new RutLsp(new Core());
  }

  /** the semantic-token legend, names in token-type index order */
  legend() {
    return JSON.parse(this.core.legend());
  }

  /** open / FULL-sync change: store the doc, answer diags + tokens + symbols */
  analyze(uri, src) {
    return JSON.parse(this.core.analyze(uri, src));
  }

  /** close: drop the doc (the workspace/std index stays) */
  forget(uri) {
    this.core.forget(uri);
  }

  /** hover at an LSP position — null when there is nothing to say */
  hover(uri, line, character) {
    return JSON.parse(this.core.hover(uri, line, character));
  }

  /**
   * completions at an LSP position. `src` — the document's CURRENT text
   * — rides the doc-sync entry (`complete_in`): the text is stored
   * before the tiers run, so the `.` the user just typed is in the text
   * they lex even while an analyze debounce is pending.
   */
  complete(uri, line, character, src) {
    if (src !== undefined) {
      return JSON.parse(this.core.complete_in(uri, src, line, character));
    }
    return JSON.parse(this.core.complete(uri, line, character));
  }

  /** go-to-definition at an LSP position — [] when nothing resolves */
  definition(uri, line, character) {
    return JSON.parse(this.core.definition(uri, line, character)) ?? [];
  }

  /** go-to-type-definition at an LSP position — [] when nothing resolves */
  typeDefinition(uri, line, character) {
    return JSON.parse(this.core.type_definition(uri, line, character)) ?? [];
  }

  /** inlay hints for a document range — [] when nothing resolves */
  inlayHint(uri, start, end) {
    return (
      JSON.parse(this.core.inlay(uri, start.line, start.character, end.line, end.character)) ?? []
    );
  }

  /** find references at an LSP position — [] when nothing resolves */
  references(uri, line, character, includeDeclaration) {
    return (
      JSON.parse(
        this.core.references(uri, line, character, includeDeclaration ? 1 : 0)
      ) ?? []
    );
  }

  /** signature help at an LSP position — null when there is none to give */
  signatureHelp(uri, line, character) {
    return JSON.parse(this.core.signature_help(uri, line, character));
  }

  /** index one workspace file (the host's stand-in for the server's fs walk) */
  addDef(uri, src) {
    this.core.add_def(uri, src);
  }

  /** parse one `rut.jsonc` — the dep table, or the error envelope */
  parseManifest(text) {
    return JSON.parse(this.core.parse_manifest(text));
  }

  /** hand the core the workspace root's `rut.lock` — the pin ledger */
  setLock(text) {
    return JSON.parse(this.core.set_lock(text));
  }

  /** hand the core the direct-dep module names (the auto-import gate) */
  setDirectDeps(names) {
    return JSON.parse(this.core.set_direct_deps(JSON.stringify(names)));
  }

  /** index one dep source file under its module NAME */
  addDefNamed(uri, name, src) {
    this.core.add_def_named(uri, name, src);
  }

  /** index one dep source file under its module NAME at a MOD PATH */
  addDefMod(uri, name, modPath, src) {
    return JSON.parse(this.core.add_def_mod(uri, name, modPath, src));
  }

  /** index one `.rutbundle` (the archive's own manifest names the module) */
  addBundle(bytes) {
    return JSON.parse(this.core.add_bundle(bytes));
  }

  /** unpack one `.rutbundle` WITHOUT indexing — JS owns the disk */
  extractBundle(bytes) {
    return JSON.parse(this.core.extract_bundle(bytes));
  }

  /** unpack one SOURCE ZIP without indexing — the source transport's twin */
  extractSrczip(bytes) {
    return JSON.parse(this.core.extract_srczip(bytes));
  }
}

/**
 * Decode the flat `SemanticTokens.data` wire array (5 u32s per token:
 * deltaLine, deltaStart, length, type-index, modifiers) into absolute
 * 0-based LSP positions. One decode, one truth — the editor overlay and
 * the headless smoke both ride this.
 *
 * @param {number[]} data the flat wire array
 * @returns {Array<{ line: number, start: number, length: number, type: number }>}
 */
export function decodeTokens(data) {
  const out = [];
  let line = 0;
  let char = 0;
  for (let i = 0; i + 4 < data.length; i += 5) {
    const deltaLine = data[i];
    line += deltaLine;
    char = deltaLine === 0 ? char + data[i + 1] : data[i + 1];
    out.push({ line, start: char, length: data[i + 2], type: data[i + 3] });
  }
  return out;
}

/**
 * Is this answer the error envelope (`{"error": …}`) rather than the
 * shape the caller asked for?
 *
 * @param {unknown} v
 * @returns {v is { error: string }}
 */
export function isEnvelope(v) {
  return typeof v === 'object' && v !== null && 'error' in v;
}
