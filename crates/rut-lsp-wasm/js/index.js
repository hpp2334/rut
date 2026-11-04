// rut-lsp-wasm's JS face — the typed binding every editor consumes.
//
// The wasm-bindgen glue (`../pkg/rut_lsp_wasm.js`, gitignored build
// output) spells every answer as a JSON string; this module is the ONE
// place that parses them and gives the shapes names. Hosts never import
// the glue directly: the VS Code extension and the demo playground both
// import THIS file, so the ABI has one runtime face and one declaration
// file (`index.d.ts`) instead of one copy per host.
//
// Every method is ASYNC (the actor seam): the call posts one message on
// the core's event loop and awaits the oneshot reply, so hosts `await`
// everything. The mpsc FIFO preserves call order — concurrent
// unresolved calls resolve in send order.
//
// The environment-neutral law: no `node:*`, no `fetch`, no DOM. `load`
// takes the artifact BYTES; who produced them (Node's fs, a browser
// fetch, an in-process cache) is the host's business. That is what lets
// the same file serve a Node extension and a browser playground. The
// host ENVIRONMENT rides the same law — injected through `load`'s
// optional second argument (`HostEnv`, declared in index.d.ts), never
// imported: the read-only capability reaches the core only because a
// host handed it over.
//
// The instance owns its session state (open docs + the index) for its
// lifetime — one `RutLsp` per editing session, mutated per request; the
// loop task holds the state exclusively.

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
   * `env` — the injected READ-ONLY host environment (`HostEnv`: read /
   * readDir / isDir / notify) — is optional: when present it rides
   * `set_env` right after the constructor (the capability the core's
   * dep walk reads); when omitted the core stays env-less, exactly the
   * shape every host boots today.
   *
   * @param {Uint8Array | ArrayBuffer} bytes the `rut-lsp.wasm` artifact
   * @param {HostEnv} [env] the read-only host environment, if the host has one
   * @returns {Promise<RutLsp>}
   */
  static async load(bytes, env) {
    await init({
      module_or_path: bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes),
    });
    const core = new Core();
    if (env !== undefined && env !== null) {
      await core.set_env(env);
    }
    return new RutLsp(core);
  }

  /** the semantic-token legend, names in token-type index order */
  async legend() {
    return JSON.parse(await this.core.legend());
  }

  /** open / FULL-sync change: store the doc, answer diags + tokens + symbols */
  async analyze(uri, src) {
    return JSON.parse(await this.core.analyze(uri, src));
  }

  /** close: drop the doc (the workspace/std index stays) */
  async forget(uri) {
    await this.core.forget(uri);
  }

  /** hover at an LSP position — null when there is nothing to say */
  async hover(uri, line, character) {
    return JSON.parse(await this.core.hover(uri, line, character));
  }

  /**
   * completions at an LSP position. `src` — the document's CURRENT text
   * — rides the doc-sync entry (`complete_in`): the text is stored
   * before the tiers run, so the `.` the user just typed is in the text
   * they lex even while an analyze debounce is pending.
   */
  async complete(uri, line, character, src) {
    if (src !== undefined) {
      return JSON.parse(await this.core.complete_in(uri, src, line, character));
    }
    return JSON.parse(await this.core.complete(uri, line, character));
  }

  /** go-to-definition at an LSP position — [] when nothing resolves */
  async definition(uri, line, character) {
    return (JSON.parse(await this.core.definition(uri, line, character)) ?? []);
  }

  /** go-to-type-definition at an LSP position — [] when nothing resolves */
  async typeDefinition(uri, line, character) {
    return (JSON.parse(await this.core.type_definition(uri, line, character)) ?? []);
  }

  /** inlay hints for a document range — [] when nothing resolves */
  async inlayHint(uri, start, end) {
    return (
      (JSON.parse(
        await this.core.inlay(uri, start.line, start.character, end.line, end.character),
      ) ?? [])
    );
  }

  /** find references at an LSP position — [] when nothing resolves */
  async references(uri, line, character, includeDeclaration) {
    return (
      (JSON.parse(
        await this.core.references(uri, line, character, includeDeclaration ? 1 : 0),
      ) ?? [])
    );
  }

  /** signature help at an LSP position — null when there is none to give */
  async signatureHelp(uri, line, character) {
    return JSON.parse(await this.core.signature_help(uri, line, character));
  }

  /** index one workspace file (the host's stand-in for the server's fs walk) */
  async addDef(uri, src) {
    await this.core.add_def(uri, src);
  }

  /** parse one `rut.jsonc` — the dep table, or the error envelope */
  async parseManifest(text) {
    return JSON.parse(await this.core.parse_manifest(text));
  }

  /** hand the core the workspace root's `rut.lock` — the pin ledger */
  async setLock(text) {
    return JSON.parse(await this.core.set_lock(text));
  }

  /** hand the core the direct-dep module names (the auto-import gate) */
  async setDirectDeps(names) {
    return JSON.parse(await this.core.set_direct_deps(JSON.stringify(names)));
  }

  /** index one dep source file under its module NAME */
  async addDefNamed(uri, name, src) {
    await this.core.add_def_named(uri, name, src);
  }

  /** index one dep source file under its module NAME at a MOD PATH */
  async addDefMod(uri, name, modPath, src) {
    return JSON.parse(await this.core.add_def_mod(uri, name, modPath, src));
  }

  /** index one `.rutbundle` (the archive's own manifest names the module) */
  async addBundle(bytes) {
    return JSON.parse(await this.core.add_bundle(bytes));
  }

  /** unpack one `.rutbundle` WITHOUT indexing — JS owns the disk */
  async extractBundle(bytes) {
    return JSON.parse(await this.core.extract_bundle(bytes));
  }

  /** unpack one SOURCE ZIP without indexing — the source transport's twin */
  async extractSrczip(bytes) {
    return JSON.parse(await this.core.extract_srczip(bytes));
  }

  /**
   * re-inject the read-only host environment (replace-on-reinject —
   * the same lane `load`'s optional `env` rides, for hosts that swap
   * the capability after boot)
   * @param {HostEnv} env
   */
  async setEnv(env) {
    await this.core.set_env(env);
  }

  /**
   * the READ-ONLY dep walk: one call, the whole walked world — each
   * root's `rut.lock` feeds the pin ledger, its `rut.jsonc` walks
   * (entries, mod trees, path rows recursing, url rows mounting from
   * the global dep store when its meta vouches the pin). The answer
   * carries the refused url rows (`missing`); the mounted world lands
   * in the core's defs store, exactly like `addDef`-indexed files.
   * @param {string[]} roots
   * @returns {Promise<WalkOut>}
   */
  async resolveDeps(roots) {
    return JSON.parse(await this.core.resolve_deps(JSON.stringify(roots)));
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

// ---- the dep engine shell (the extension's walk scheduler, moved
// into the crate face — timers are neutral) ----

/**
 * The dep engine: debounce + one-run-in-flight + rerun-once-if-stirred
 * — the old extension's `walking`/`walkRerun` law, environment-neutral
 * (`setTimeout` + `Promise` only; no `node:*`, no DOM). Each run is
 * one `resolveDeps(roots())` — `roots` is a HOST callback re-read at
 * every run, so the engine always walks the world the host sees now.
 *
 * `stir()` arms (or re-arms) the debounce; a stir that lands while a
 * run is in flight coalesces into ONE rerun after it settles. `env`,
 * when present, re-injects the read-only host environment before each
 * run (replace-on-reinject — a host may hand over a fresh capability
 * per walk). `dispose()` retires the engine: pending timers clear and
 * later stirs are no-ops.
 *
 * @param {RutLsp} rut the loaded core (env-carrying or not)
 * @param {{ env?: HostEnv, roots: () => string[], debounceMs?: number }} opts
 * @returns {{ stir(): void, dispose(): void }}
 */
export function depEngine(rut, opts) {
  const { env, roots, debounceMs = 200 } = opts;
  let timer;
  let running = false;
  let rerun = false;
  let disposed = false;
  const fire = async () => {
    if (running) {
      rerun = true; // a stir landed mid-run — ONE rerun when it settles
      return;
    }
    running = true;
    try {
      if (env !== undefined && env !== null) {
        await rut.setEnv(env);
      }
      await rut.resolveDeps(roots());
    } finally {
      running = false;
      if (rerun && !disposed) {
        rerun = false;
        stir(); // back through the debounce, exactly the old law
      }
    }
  };
  function stir() {
    if (disposed) {
      return;
    }
    if (timer !== undefined) {
      clearTimeout(timer);
    }
    timer = setTimeout(() => {
      timer = undefined;
      void fire();
    }, debounceMs);
  }
  function dispose() {
    disposed = true;
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
  }
  return { stir, dispose };
}
