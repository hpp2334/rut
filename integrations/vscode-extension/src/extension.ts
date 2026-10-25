// rut-vscode — runs the language core as an in-process wasm module
// (rut-lsp-wasm raw ABI, RFC 0041 §2): no server process, no per-platform
// binaries — one .wasm serves every platform. Providers register directly
// against vscode.languages; the same pure queries serve the native
// `rut-lsp` stdio server for editors that speak LSP. Without
// bin/rut-lsp.wasm the TextMate grammar still colors the basics and an
// info message points at `npm run build:wasm`.

import * as crypto from 'node:crypto';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { Analysis, DepRow, isEnvelope, LspCompletionItem, LspDiag, LspInlayHint, LspLocation, LspRange, LspSignatureHelp, LspSymbol, RutWasm, SrczipExtracted } from './wasm';

const SELECTOR: vscode.DocumentSelector = { language: 'rut' };

// keystrokes coalesce: the last change inside the window wins (the
// stdio server got this from the language client's batching)
const ANALYSIS_DEBOUNCE_MS = 200;

// mirrors the native server's workspace-scan cap — an LSP is a guest,
// not an indexer daemon
const MAX_WORKSPACE_FILES = 500;

// the manifest's one name — one directory is one module
const MANIFEST_NAME = 'rut.jsonc';

// the dep walk's budget beside the 500-file workspace cap: depth for
// the recursion, files for everything the walk indexes
const MAX_DEP_DEPTH = 8;
const MAX_DEP_FILES = 200;

// the fetch cap — the same bytes ceiling the CLI's remote enforces
const MAX_FETCH_BYTES = 256 * 1024 * 1024;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const wasmPath = path.join(context.extensionPath, 'bin', 'rut-lsp.wasm');
  if (!fs.existsSync(wasmPath)) {
    vscode.window.showInformationMessage(
      'rut-lsp.wasm not found — run `npm run build:wasm` in integrations/vscode-extension ' +
        'for semantic highlighting, diagnostics, symbols, hover and completions. ' +
        'Basic grammar highlighting stays on.'
    );
    return;
  }
  const rut = await RutWasm.load(wasmPath);
  registerAll(context, rut);
}

function registerAll(context: vscode.ExtensionContext, rut: RutWasm): void {
  const subscriptions = context.subscriptions;

  // ---- pushed diagnostics (the wasm twin of publish_diagnostics) ----
  const diagnostics = vscode.languages.createDiagnosticCollection('rut');
  const pending = new Map<string, ReturnType<typeof setTimeout>>();
  const publish = (doc: vscode.TextDocument): void => {
    diagnostics.set(doc.uri, analyzeDoc(rut, doc).diags.map(toDiagnostic));
  };
  const schedule = (doc: vscode.TextDocument): void => {
    if (doc.languageId !== 'rut') {
      return;
    }
    const uri = doc.uri.toString();
    const timer = pending.get(uri);
    if (timer !== undefined) {
      clearTimeout(timer);
    }
    pending.set(
      uri,
      setTimeout(() => {
        pending.delete(uri);
        if (!doc.isClosed) {
          publish(doc);
        }
      }, ANALYSIS_DEBOUNCE_MS)
    );
  };
  subscriptions.push(
    diagnostics,
    vscode.workspace.onDidOpenTextDocument(schedule),
    vscode.workspace.onDidChangeTextDocument((event) => schedule(event.document)),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      const uri = doc.uri.toString();
      const timer = pending.get(uri);
      if (timer !== undefined) {
        clearTimeout(timer);
        pending.delete(uri);
      }
      rut.forget(uri);
      diagnostics.delete(doc.uri);
    })
  );
  // documents already open at activation get their analysis now
  for (const doc of vscode.workspace.textDocuments) {
    schedule(doc);
  }

  // ---- the language features ----
  subscriptions.push(
    registerSemanticTokens(rut),
    registerDocumentSymbols(rut),
    registerHover(rut),
    registerCompletion(rut),
    registerDefinition(rut),
    registerTypeDefinition(rut),
    registerReferences(rut),
    registerInlayHints(rut),
    registerSignatureHelp(rut)
  );

  // the palette's manual re-index — the same lane activation and the
  // folder-change hook ride, on demand: covers watcher misses (a mount
  // a glob never saw, a walk that raced an edit)
  subscriptions.push(
    vscode.commands.registerCommand('rut.reindexDeps', () => {
      void indexWorkspace(rut).then(() => scheduleDepWalk(rut));
    })
  );

  // workspace index off the hot activation path — hover/completion may
  // briefly resolve against the std surface + open docs only. The dep
  // walk rides after it: the root `rut.jsonc`'s rows mount the deps
  // (path dirs, extracted pinned url bundles) into the same index.
  watchDeps(context, rut);
  void indexWorkspace(rut).then(() => scheduleDepWalk(rut));
}

// ---- deps: the manifest walk ----
//
// After the workspace scan each workspace folder's root `rut.jsonc` is
// handed to the wasm (`rut_parse_manifest` — JS moves bytes, the wasm
// parses; law: no JS-side rut parsing). The folder's `rut.lock` rides
// in first (`rut_set_lock`): its url→pin ledger is what the parse
// stamps onto the url rows — the pin's only source. Path deps
// resolve against the manifest dir — OUTSIDE the workspace folder is
// the point — and recurse cycle-safe under a depth/file budget. Url
// deps ride the shared `.rut/cache/` (the Rust-computed cache_path,
// the same dir the CLI fills): cache-first, a hit verified against the
// sha256 pin (a poisoned entry evicts and re-fetches — the CLI's
// healing law), a miss GETs, pins the bytes BEFORE they land anywhere,
// and writes the cache atomically (tmp + rename). The verified bytes
// then EXTRACT into real files under `.rut/deps/<module>-<pin12>/`
// (content-addressed by the byte pin) and re-index through the same
// lane path deps use — go-to-definition lands in openable `file://`
// paths, no `bundle:` origins. No pin, no bytes — one error hint.
// Every failure is ONE hint per dep naming the CLI alternative: never
// spam, never silent wrong results.

interface WalkCtx {
  rut: RutWasm;
  /// the anchoring workspace folder — where `.rut/cache/` lives
  folder: vscode.Uri;
  /// resolved manifest dirs — the cycle break
  seen: Set<string>;
  /// mounted module names — first mount wins (the native gate: a name
  /// an earlier row or embedded group mounted never re-mounts from a
  /// ride-along tree, so a diamond's pins cannot double-stamp)
  mounted: Set<string>;
  /// remaining dep-file budget
  files: number;
  /// the `.rut/deps/` trees this walk's rows reference (fsPaths) —
  /// everything else under the root is a stale pin bump, swept
  depDirs: Set<string>;
  /// the walk hit a budget (files or depth) — its referenced set is
  /// incomplete, the stale sweep stays out of the way
  truncated: boolean;
}

// hints are once-per-reason per session: a watcher re-walk fires
// dozens of times while the network is down — the first hint names the
// fix, the rest would be spam
const depHintsShown = new Set<string>();

function hintOnce(key: string, message: string, error: boolean): void {
  if (depHintsShown.has(key)) {
    return;
  }
  depHintsShown.add(key);
  const shown = error
    ? vscode.window.showErrorMessage(message)
    : vscode.window.showInformationMessage(message);
  void Promise.resolve(shown).catch(() => undefined);
}

// the dep walk is debounced like the analysis (watchers fire in
// bursts), serialized (a walk in flight outlives its trigger), and
// re-run once when triggers arrived mid-walk
let walkTimer: ReturnType<typeof setTimeout> | undefined;
let walking = false;
let walkRerun = false;

function scheduleDepWalk(rut: RutWasm): void {
  if (walkTimer !== undefined) {
    clearTimeout(walkTimer);
  }
  walkTimer = setTimeout(() => {
    walkTimer = undefined;
    void runDepWalk(rut);
  }, ANALYSIS_DEBOUNCE_MS);
}

async function runDepWalk(rut: RutWasm): Promise<void> {
  if (walking) {
    walkRerun = true;
    return;
  }
  walking = true;
  try {
    await walkDeps(rut);
  } finally {
    walking = false;
    if (walkRerun) {
      walkRerun = false;
      scheduleDepWalk(rut);
    }
  }
}

// `**/rut.jsonc` + `**/mod.rut` + `.rut/cache/**` re-run the walk (a
// mod tree landing or moving re-mounts it); a workspace-folder change
// restarts the whole index (watchers re-arm over the new folder set,
// the scan + walk re-run — re-indexing is replace-by-origin, so this
// is idempotent)
function watchDeps(context: vscode.ExtensionContext, rut: RutWasm): void {
  const watchers: vscode.Disposable[] = [];
  const watchFolder = (folder: vscode.WorkspaceFolder): void => {
    for (const pattern of [`**/${MANIFEST_NAME}`, '**/mod.rut', '.rut/cache/**']) {
      const watcher = vscode.workspace.createFileSystemWatcher(new vscode.RelativePattern(folder, pattern));
      watchers.push(
        watcher,
        watcher.onDidChange(() => scheduleDepWalk(rut)),
        watcher.onDidCreate(() => scheduleDepWalk(rut)),
        watcher.onDidDelete(() => scheduleDepWalk(rut))
      );
    }
  };
  const rearm = (): void => {
    for (const w of watchers) {
      w.dispose();
    }
    watchers.length = 0;
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
      watchFolder(folder);
    }
  };
  rearm();
  context.subscriptions.push(
    vscode.workspace.onDidChangeWorkspaceFolders(() => {
      rearm();
      void indexWorkspace(rut).then(() => scheduleDepWalk(rut));
    }),
    { dispose: () => watchers.forEach((w) => w.dispose()) }
  );
}

async function walkDeps(rut: RutWasm): Promise<void> {
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    // the pin ledger rides the walk root: `rut.lock` beside the root
    // manifest feeds `rut.setLock` BEFORE the manifest parses, so its
    // url rows carry the lock's pins. A miss hands over empty bytes —
    // clearing a previous folder's ledger (multi-root precedence:
    // folder order decides; url rows the lock pins live in ROOT
    // manifests — accepted), and the error envelope that comes back is
    // expected, not a hint. A PRESENT-but-malformed lock is the one
    // error hint; the empty ledger stands (the honest unpinned
    // refusal, never stale or guessed pins).
    const lockUri = vscode.Uri.joinPath(folder.uri, 'rut.lock');
    const lockBytes = await readFileNullable(lockUri);
    const fed = rut.setLock(lockBytes === null ? '' : new TextDecoder().decode(lockBytes));
    if (lockBytes !== null && isEnvelope(fed)) {
      hintOnce(`lock:${lockUri.fsPath}:${fed.error}`, `rut: ${lockUri.fsPath} — ${fed.error}`, true);
    }
    const ctx: WalkCtx = {
      rut,
      folder: folder.uri,
      seen: new Set(),
      mounted: new Set(),
      files: MAX_DEP_FILES,
      depDirs: new Set(),
      truncated: false,
    };
    await walkManifest(ctx, vscode.Uri.joinPath(folder.uri, MANIFEST_NAME), 1);
    await sweepStaleDepTrees(ctx);
  }
}

// pin bumps leave sibling trees behind — the walk deletes the
// `.rut/deps/<dir>` trees no current manifest row referenced (the dir
// stays content-addressed, never growing across bumps). A truncated
// walk skips the sweep: a budget miss is not evidence a tree is stale.
async function sweepStaleDepTrees(ctx: WalkCtx): Promise<void> {
  if (ctx.truncated) {
    return;
  }
  const depsRoot = vscode.Uri.joinPath(ctx.folder, '.rut', 'deps');
  let names: Array<[string, vscode.FileType]>;
  try {
    names = await vscode.workspace.fs.readDirectory(depsRoot);
  } catch {
    return; // no deps dir (or unreadable) — nothing to sweep
  }
  for (const [name, type] of names) {
    if (type !== vscode.FileType.Directory || ctx.depDirs.has(path.join(depsRoot.fsPath, name))) {
      continue;
    }
    try {
      await vscode.workspace.fs.delete(vscode.Uri.joinPath(depsRoot, name), { recursive: true });
    } catch {
      // a concurrent reader wins this race — the next walk retries
    }
  }
}

// one manifest: its own entry sources index under the module name (the
// uri is the file's real on-disk URI so F12 jumps land), then its dep
// rows walk — `deps` at any depth, `dev-deps` only at the root (the
// loader's law: a dep's dev table never enters a consumer's world).
// Namespaced/const-carrying path deps (calc's `Math`) index their
// surface only — namespace/consts have no voice in
// `rut_add_def_named`/`rut_add_def_mod`; the extracted-tree lane
// indexes the surface the same way.
async function walkManifest(ctx: WalkCtx, manifestUri: vscode.Uri, depth: number): Promise<void> {
  if (depth > MAX_DEP_DEPTH || ctx.files <= 0) {
    ctx.truncated = true; // the referenced set is incomplete — the sweep skips
    return;
  }
  const dir = vscode.Uri.joinPath(manifestUri, '..');
  const dirKey = dir.fsPath;
  if (ctx.seen.has(dirKey)) {
    return;
  }
  ctx.seen.add(dirKey);
  const bytes = await readFileNullable(manifestUri);
  if (bytes === null) {
    return; // no manifest here — nothing to parse, nothing to hint
  }
  const table = ctx.rut.parseManifest(new TextDecoder().decode(bytes));
  if (isEnvelope(table)) {
    hintOnce(
      `manifest:${dirKey}:${table.error}`,
      `rut: ${path.basename(manifestUri.fsPath)} — ${table.error}`,
      true
    );
    return;
  }
  const module = table.name ?? path.basename(dirKey);
  // the ROOT manifest's own rows are the doc's DIRECT deps — the
  // compiler's closure check (`Session::resolve_use` → `declares`)
  // refuses `use` of a mounted-but-undeclared package, so the auto-
  // import tier gates on this set: hand it to the wasm BEFORE mounting
  // anything (the dep indexes stamp the marker at index time). The
  // doc's own package name rides too — uses may spell it. Names
  // accumulate in the wasm's set (multi-folder walks, re-walks).
  if (depth === 1) {
    const stamped = ctx.rut.setDirectDeps([
      ...(table.name ? [table.name] : []),
      ...table.deps.map((r) => r.name),
      ...table.dev_deps.map((r) => r.name),
    ]);
    if (isEnvelope(stamped)) {
      hintOnce(`direct:${dirKey}:${stamped.error}`, `rut: ${stamped.error}`, true);
    }
  }
  // first mount wins — the native gate: a module name an earlier row
  // (or embedded group) mounted never re-mounts, so a diamond's
  // different-pin copies cannot double-stamp the world
  if (ctx.mounted.has(module)) {
    return;
  }
  ctx.mounted.add(module);
  for (const entry of table.entries) {
    if (ctx.files <= 0) {
      ctx.truncated = true;
      return;
    }
    const srcUri = vscode.Uri.joinPath(dir, entry.path);
    const src = await readFileNullable(srcUri);
    if (src === null) {
      continue; // a manifest naming a missing entry — index what exists
    }
    ctx.files--;
    ctx.rut.addDefNamed(srcUri.toString(), module, new TextDecoder().decode(src));
  }
  // the mod tree (file modules): the root module's `mod NAME;` edges
  // mount `NAME/mod.rut` recursively — the wasm parses every file
  // (`addDefMod` answers the declared children), JS only moves bytes.
  // The root is `src/mod.rut` under the shell (the dep table spells
  // it as the Impl entry, indexed by the entries loop at mod path '')
  const entryPaths = table.entries.filter((e) => e.mode === 'Impl').map((e) => e.path);
  await mountModTree(ctx, dir, module, entryPaths, depth);
  const rows: DepRow[] = [
    ...table.deps,
    ...(depth === 1 ? table.dev_deps : []),
  ];
  for (const row of rows) {
    if (ctx.files <= 0) {
      ctx.truncated = true;
      return;
    }
    if (row.source.kind === 'url') {
      await mountUrlDep(ctx, row, depth);
      continue;
    }
    const depDir = path.resolve(dirKey, row.source.dir);
    const depManifestUri = vscode.Uri.file(path.join(depDir, MANIFEST_NAME));
    if (!fs.existsSync(depManifestUri.fsPath)) {
      hintOnce(
        `path:${dirKey}:${row.name}`,
        `rut: dep '${row.name}' — ${row.source.dir} has no ${MANIFEST_NAME}; ` +
          `a path dep is a module directory (one manifest per module)`,
        true
      );
      continue;
    }
    await walkManifest(ctx, depManifestUri, depth + 1);
  }
}

// one package's mod tree: `mod NAME;` declarations are the graph, so
// the walk follows the wasm-parsed decls (`addDefMod` returns them) —
// the child edge mounts `NAME/mod.rut` beside the declaring file, then
// the single-file spelling `NAME.rut`, recursively, depth- and
// file-budget-capped. A read miss names the dep once (the loader's
// loud diagnostics, mirrored — never silent missing modules).
async function mountModTree(
  ctx: WalkCtx,
  pkgDir: vscode.Uri,
  module: string,
  entryPaths: string[],
  depth: number
): Promise<void> {
  if (depth > MAX_DEP_DEPTH || ctx.files <= 0) {
    ctx.truncated = true; // the walk is incomplete — the sweep skips
    return;
  }
  let rootUri: vscode.Uri;
  let text: string;
  let decls: Array<{ name: string; vis: 'pub' | 'mod' }>;
  if (entryPaths.length > 0) {
    // the root-module lane: `src/mod.rut` is the root (already indexed by
    // the entries loop — its text is re-read for the walk; the
    // re-stamp at mod path '' is the same answer `None` spelled)
    rootUri = vscode.Uri.joinPath(pkgDir, entryPaths[0]);
    const bytes = await readFileNullable(rootUri);
    if (bytes === null) {
      return;
    }
    text = new TextDecoder().decode(bytes);
    const stamped = ctx.rut.addDefMod(rootUri.toString(), module, '', text);
    if (isEnvelope(stamped)) {
      hintOnce(`mods:${pkgDir.fsPath}`, `rut: ${module} — ${stamped.error}`, true);
      return;
    }
    decls = stamped.decls;
  } else {
    // a host pkg (or surface-only dev state): no body row — `src/mod.rut`
    // under the shell is probed anyway; a read miss mounts nothing
    // (the ROOT is always the directory form — only child edges carry
    // the `NAME.rut` spelling)
    rootUri = vscode.Uri.joinPath(pkgDir, 'src/mod.rut');
    const bytes = await readFileNullable(rootUri);
    if (bytes === null) {
      return; // no root module — nothing mounts (surface-only dev state)
    }
    text = new TextDecoder().decode(bytes);
    ctx.files--;
    const stamped = ctx.rut.addDefMod(rootUri.toString(), module, '', text);
    if (isEnvelope(stamped)) {
      hintOnce(`mods:${pkgDir.fsPath}`, `rut: ${module} — ${stamped.error}`, true);
      return;
    }
    decls = stamped.decls;
  }
  // the children mount BESIDE THE ROOT FILE — inside the package's
  // `src/` shell
  const rootDir = vscode.Uri.file(path.dirname(rootUri.fsPath));
  for (const decl of decls) {
    if (ctx.files <= 0) {
      ctx.truncated = true;
      return;
    }
    await mountChildMod(ctx, rootDir, module, decl.name, depth);
  }
}

// one `mod NAME;` edge: `NAME/mod.rut` beside the declaring file
// first, then the single-file spelling `NAME.rut` — both INSIDE the
// declaring package's `src/` shell (the root's children probe beside
// the root file); BOTH present is the loader's
// loud ambiguity (hinted once, mirrored). The found child stamps under
// the module and its own decls recurse from its physical directory
// (a file-form child's children mount beside it, not under `NAME/`).
async function mountChildMod(
  ctx: WalkCtx,
  declaringDir: vscode.Uri,
  module: string,
  name: string,
  depth: number
): Promise<void> {
  const dirUri = vscode.Uri.joinPath(declaringDir, name, 'mod.rut');
  const fileUri = vscode.Uri.joinPath(declaringDir, `${name}.rut`);
  const [dirBytes, fileBytes] = await Promise.all([
    readFileNullable(dirUri),
    readFileNullable(fileUri)
  ]);
  let childUri: vscode.Uri;
  let bytes: Uint8Array;
  if (dirBytes !== null && fileBytes !== null) {
    hintOnce(
      `mods:${fileUri.fsPath}`,
      `rut: ${module} — both ${name}/mod.rut and ${name}.rut exist; keep one`,
      true
    );
    return;
  } else if (dirBytes !== null) {
    childUri = dirUri;
    bytes = dirBytes;
  } else if (fileBytes !== null) {
    childUri = fileUri;
    bytes = fileBytes;
  } else {
    return; // nothing under either spelling — the edge mounts nothing
  }
  if (depth + 1 > MAX_DEP_DEPTH || ctx.files <= 0) {
    ctx.truncated = true;
    return;
  }
  ctx.files--;
  const text = new TextDecoder().decode(bytes);
  const stamped = ctx.rut.addDefMod(childUri.toString(), module, '', text);
  if (isEnvelope(stamped)) {
    hintOnce(`mods:${declaringDir.fsPath}`, `rut: ${module} — ${stamped.error}`, true);
    return;
  }
  const childDir = childUri.fsPath === dirUri.fsPath
    ? vscode.Uri.joinPath(declaringDir, name)
    : declaringDir;
  for (const decl of stamped.decls) {
    if (ctx.files <= 0) {
      ctx.truncated = true;
      return;
    }
    await mountChildMod(ctx, childDir, module, decl.name, depth + 1);
  }
}

// one url row — the mount door. `sha256`/`cache_path` are Rust-computed
// (the row's envelope carries them; JS never hashes URLs). The
// pin-verified bytes EXTRACT into real files under
// `.rut/deps/<module>-<pin12>/` — content-addressed by the byte pin
// (same pin ⇒ same tree, a bump ⇒ a sibling the stale sweep prunes) —
// and index through the same lane path deps use (`addDefMod` /
// `addDefNamed` over `file://` uris): go-to-definition lands in
// openable workspace files, no `bundle:` origins.
async function mountUrlDep(ctx: WalkCtx, row: DepRow, depth: number): Promise<void> {
  const source = row.source;
  if (source.kind !== 'url') {
    return;
  }
  const { url, sha256, cache_path } = source;
  if (sha256 === null) {
    hintOnce(
      `unpinned:${url}`,
      `rut: dep '${row.name}' has no sha256 pin in rut.lock — the editor never mounts unpinned ` +
        `url bytes; run \`rut install\` in the project to (re)pin, then reload the window`,
      true
    );
    return;
  }
  // the tree's name rides the PIN: the row references this dir whether
  // or not this walk's fetch/extract succeeds — a transient failure
  // keeps the last good tree instead of sweeping it
  const depDir = vscode.Uri.joinPath(ctx.folder, '.rut', 'deps', `${row.name}-${sha256.slice(0, 12)}`);
  ctx.depDirs.add(depDir.fsPath);
  const cacheUri = vscode.Uri.joinPath(ctx.folder, cache_path);
  let bytes = await readFileNullable(cacheUri);
  if (bytes !== null && (await sha256Hex(bytes)) !== sha256) {
    // poisoned entry: the pin is checked at the mount door on every
    // load — evict so the refetch below heals it (the CLI's law)
    try {
      await vscode.workspace.fs.delete(cacheUri);
    } catch {
      // already gone — the refetch decides
    }
    bytes = null;
  }
  if (bytes === null) {
    try {
      const resp = await fetch(url);
      if (!resp.ok) {
        throw new Error(`HTTP ${resp.status}`);
      }
      const buf = new Uint8Array(await resp.arrayBuffer());
      if (buf.byteLength > MAX_FETCH_BYTES) {
        throw new Error(`${buf.byteLength} bytes exceeds the fetch cap (${MAX_FETCH_BYTES})`);
      }
      if ((await sha256Hex(buf)) !== sha256) {
        hintOnce(
          `pin:${url}`,
          `rut: dep '${row.name}' — sha256 pin mismatch for ${url}: the manifest pins ` +
            `${sha256}, the fetched bytes hash differently — refusing to mount`,
          true
        );
        return;
      }
      await atomicCacheWrite(cacheUri, buf);
      bytes = buf;
    } catch (e) {
      hintOnce(
        `fetch:${url}`,
        `rut: dep '${row.name}' (${url}) could not be fetched ` +
          `(${e instanceof Error ? e.message : String(e)}) — run \`rut fetch\` in the ` +
          `project to warm .rut/cache, then reload the window`,
        false
      );
      return;
    }
  }
  // the artifact kind the url's own spelling declares — a `.zip` is
  // the source transport (its `src/**` tree extracts like a bundle's
  // ride-along set), anything else the compiled wire. (The old
  // capability seam — a loaded wasm predating the transport — is
  // structurally gone: the wasm-bindgen glue and the .wasm ship as one
  // pair in the package, so the module cannot be older than its lane.)
  const isZip = /\.zip$/.test(cache_path);
  const extracted = isZip ? ctx.rut.extractSrczip(bytes) : ctx.rut.extractBundle(bytes);
  if (isEnvelope(extracted)) {
    hintOnce(
      `bundle:${url}`,
      `rut: dep '${row.name}' — the bundle did not mount: ${extracted.error}`,
      true
    );
    return;
  }
  if (isZip) {
    // THE UNPACK LAW (the native `mount_zip_dep`'s, mirrored): the tree
    // lays down WHOLE — the archive's own manifest beside the files,
    // the embedded `deps/<identity>/` groups under it (a source zip's
    // transitive pkgs: the host surface `ink_host` inside `ink.zip`,
    // the host pkgs every scaffold never spells) — then the ordinary
    // dir lane mounts the pkg: the entry law stamps the surfaces, the
    // declared mod tree follows the wasm-parsed decls, and the pkg's
    // own dep rows recurse INTO the embedded groups (path rows beside
    // the manifest). First mount wins in `walkManifest`'s gate, so a
    // name the walk already mounted never re-mounts from a ride-along.
    const tree = extracted as SrczipExtracted;
    const writes: Array<{ rel: string; src: string }> = [
      { rel: MANIFEST_NAME, src: tree.manifest },
      ...tree.files.map((f) => ({ rel: f.path, src: f.src })),
      ...tree.embedded.map((e) => ({ rel: e.path, src: e.src })),
    ];
    for (const w of writes) {
      // real files on real drift only — an identical tree stays
      // untouched, or the `**/rut.jsonc` watcher's re-walk would
      // rewrite it forever
      const target = vscode.Uri.joinPath(depDir, w.rel);
      if (!(await fileHasContent(target, w.src))) {
        await vscode.workspace.fs.createDirectory(vscode.Uri.joinPath(target, '..'));
        await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(w.src));
      }
    }
    // the pkg's OWN files index directly — the bundle lane's law (the
    // published zip manifest carries no entry rows, so `walkManifest`'s
    // entries loop cannot see them; the mod paths ride the envelope).
    // First mount wins, the native gate: a name an earlier row mounted
    // never re-mounts from a ride-along.
    if (!ctx.mounted.has(tree.module)) {
      ctx.mounted.add(tree.module);
      for (const f of tree.files) {
        if (ctx.files <= 0) {
          ctx.truncated = true;
          return;
        }
        const fileUri = vscode.Uri.joinPath(depDir, f.path);
        if (f.mod_path === null) {
          ctx.rut.addDefNamed(fileUri.toString(), tree.module, f.src);
        } else {
          const stamped = ctx.rut.addDefMod(fileUri.toString(), tree.module, f.mod_path, f.src);
          if (isEnvelope(stamped)) {
            hintOnce(
              `bundle:${url}`,
              `rut: dep '${row.name}' — ${tree.module} did not index: ${stamped.error}`,
              true
            );
            return;
          }
        }
        ctx.files--;
      }
    }
    await walkManifest(ctx, vscode.Uri.joinPath(depDir, MANIFEST_NAME), depth + 1);
    return;
  }
  for (const f of extracted.files) {
    if (ctx.files <= 0) {
      ctx.truncated = true;
      return;
    }
    // the file lands at its archive path (a `.d.rut` stays `.d.rut`,
    // so the uri convention's mode matches the envelope's) — but only
    // on real drift: an identical file stays untouched, or the
    // `**/mod.rut` watcher's re-walk would rewrite it forever
    const fileUri = vscode.Uri.joinPath(depDir, f.path);
    if (!(await fileHasContent(fileUri, f.src))) {
      await vscode.workspace.fs.createDirectory(vscode.Uri.joinPath(fileUri, '..'));
      await vscode.workspace.fs.writeFile(fileUri, new TextEncoder().encode(f.src));
    }
    ctx.files--;
    // the same lane path deps use — mod-path files through
    // `addDefMod` (the envelope spells the path: `''` the root,
    // `<path>` a child), flat files under the module NAME
    if (f.mod_path === null) {
      ctx.rut.addDefNamed(fileUri.toString(), extracted.module, f.src);
      continue;
    }
    const stamped = ctx.rut.addDefMod(fileUri.toString(), extracted.module, f.mod_path, f.src);
    if (isEnvelope(stamped)) {
      hintOnce(
        `bundle:${url}`,
        `rut: dep '${row.name}' — ${extracted.module} did not index: ${stamped.error}`,
        true
      );
      return;
    }
  }
}

// the sha256 pin check — Node's webcrypto, hex-lowercase like the
// manifest's normalized pin
async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.webcrypto.subtle.digest('SHA-256', bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

// the CLI's atomic entry write: tmp beside the target, then rename —
// a reader never sees a partial entry
async function atomicCacheWrite(target: vscode.Uri, bytes: Uint8Array): Promise<void> {
  await vscode.workspace.fs.createDirectory(vscode.Uri.joinPath(target, '..'));
  const tmp = target.with({ path: `${target.path}.part` });
  await vscode.workspace.fs.writeFile(tmp, bytes);
  await vscode.workspace.fs.rename(tmp, target, { overwrite: true });
}

// a missing or unreadable file is a null, not a failure — callers
// decide whether that's a miss (cache) or a hint (dep)
async function readFileNullable(file: vscode.Uri): Promise<Uint8Array | null> {
  try {
    return await vscode.workspace.fs.readFile(file);
  } catch {
    return null;
  }
}

// does the file exist with EXACTLY this text? — the extracted-tree
// writes ride it: an unchanged tree never rewrites, so the watchers'
// re-walk settles (the first mount writes, a drifted file heals)
async function fileHasContent(file: vscode.Uri, src: string): Promise<boolean> {
  const bytes = await readFileNullable(file);
  return bytes !== null && new TextDecoder().decode(bytes) === src;
}

// ---- providers ----

function registerSemanticTokens(rut: RutWasm): vscode.Disposable {
  const legend = new vscode.SemanticTokensLegend(rut.legend(), []);
  return vscode.languages.registerDocumentSemanticTokensProvider(
    SELECTOR,
    {
      provideDocumentSemanticTokens(doc: vscode.TextDocument): vscode.SemanticTokens {
        const data = analyzeDoc(rut, doc).tokens.data;
        const builder = new vscode.SemanticTokensBuilder(legend);
        // decode the LSP delta encoding into the builder's absolute slots
        let line = 0;
        let char = 0;
        for (let i = 0; i + 4 < data.length; i += 5) {
          const deltaLine = data[i];
          line += deltaLine;
          char = deltaLine === 0 ? char + data[i + 1] : data[i + 1];
          builder.push(line, char, data[i + 2], data[i + 3]);
        }
        return builder.build();
      },
    },
    legend
  );
}

function registerDocumentSymbols(rut: RutWasm): vscode.Disposable {
  return vscode.languages.registerDocumentSymbolProvider(SELECTOR, {
    provideDocumentSymbols(doc: vscode.TextDocument): vscode.DocumentSymbol[] {
      return analyzeDoc(rut, doc).symbols.map(toSymbol);
    },
  });
}

function registerHover(rut: RutWasm): vscode.Disposable {
  return vscode.languages.registerHoverProvider(SELECTOR, {
    provideHover(doc: vscode.TextDocument, position: vscode.Position): vscode.Hover | undefined {
      const h = rut.hover(doc.uri.toString(), position.line, position.character);
      if (!h) {
        return undefined;
      }
      return new vscode.Hover(
        new vscode.MarkdownString(h.contents.value),
        h.range ? toRange(h.range) : undefined
      );
    },
  });
}

function registerCompletion(rut: RutWasm): vscode.Disposable {
  return vscode.languages.registerCompletionItemProvider(
    SELECTOR,
    {
      provideCompletionItems(
        doc: vscode.TextDocument,
        position: vscode.Position
      ): vscode.CompletionItem[] {
        // the CURRENT text rides the request (the doc-sync entry):
        // completion fires on the keystroke while the analyze lane
        // debounces ~200 ms — the stored doc may still be short the `.`
        // this very request is about
        return rut.complete(doc.uri.toString(), position.line, position.character, doc.getText()).map(toCompletionItem);
      },
    },
    '.', // trigger: member completion after a receiver dot
    ':', // trigger: the `::` tiers (module members, enum members,
    // namespace statics, associated fns) and the type tier at an
    // annotation boundary (`let x: ⏐`), both on the colon key
    '{', // trigger: the brace form of a use (`use pouch::{ ⏐`)
  );
}

// ctrl+click / F12 — the wasm definition query, targets resolved to
// real URIs (the embedded std surface jumps carry repo-relative paths
// into the true rut/ sources)
function registerDefinition(rut: RutWasm): vscode.Disposable {
  return vscode.languages.registerDefinitionProvider(SELECTOR, {
    provideDefinition(
      doc: vscode.TextDocument,
      position: vscode.Position
    ): vscode.Location[] {
      return resolveLocations(
        rut,
        rut.definition(doc.uri.toString(), position.line, position.character),
        doc.uri.toString()
      );
    },
  });
}

function registerTypeDefinition(rut: RutWasm): vscode.Disposable {
  return vscode.languages.registerTypeDefinitionProvider(SELECTOR, {
    provideTypeDefinition(
      doc: vscode.TextDocument,
      position: vscode.Position
    ): vscode.Location[] {
      return resolveLocations(
        rut,
        rut.typeDefinition(doc.uri.toString(), position.line, position.character),
        doc.uri.toString()
      );
    },
  });
}

// the inline inference display — the wasm core supplies position +
// label + the colon (type hints after unannotated bindings, param
// names before exact-arity call args); VS Code renders the dotted
// decoration and `editor.inlayHints` toggles it
function registerInlayHints(rut: RutWasm): vscode.Disposable {
  return vscode.languages.registerInlayHintsProvider(SELECTOR, {
    provideInlayHints(doc: vscode.TextDocument, range: vscode.Range): vscode.InlayHint[] {
      return rut
        .inlayHint(doc.uri.toString(), range.start, range.end)
        .map(toInlayHint);
    },
  });
}

// Shift+F12 / Find All References — the wasm references query (the
// definition index read backwards); the LSP includeDeclaration toggle
// rides the vscode ReferenceContext through
function registerReferences(rut: RutWasm): vscode.Disposable {
  return vscode.languages.registerReferenceProvider(SELECTOR, {
    provideReferences(
      doc: vscode.TextDocument,
      position: vscode.Position,
      context: vscode.ReferenceContext
    ): vscode.Location[] {
      return resolveLocations(
        rut,
        rut.references(doc.uri.toString(), position.line, position.character, context.includeDeclaration),
        doc.uri.toString()
      );
    },
  });
}

// signature help (`(` / `,` triggers) — the callee resolves through the
// same machinery the param-name hints ride; a mismatch shows nothing
function registerSignatureHelp(rut: RutWasm): vscode.Disposable {
  return vscode.languages.registerSignatureHelpProvider(
    SELECTOR,
    {
      provideSignatureHelp(
        doc: vscode.TextDocument,
        position: vscode.Position
      ): vscode.SignatureHelp | undefined {
        const h = rut.signatureHelp(doc.uri.toString(), position.line, position.character);
        return h ? toSignatureHelp(h) : undefined;
      },
    },
    '(',
    ','
  );
}

// ---- workspace index (the wasm stand-in for the server's fs walk) ----

// the folder-level package's name (its manifest parses in the wasm) —
// `null` when the folder has no manifest or a broken one
async function folderPkgName(rut: RutWasm, folder: vscode.Uri): Promise<string | null> {
  const bytes = await readFileNullable(vscode.Uri.joinPath(folder, MANIFEST_NAME));
  if (bytes === null) {
    return null;
  }
  const table = rut.parseManifest(new TextDecoder().decode(bytes));
  return isEnvelope(table) ? null : table.name;
}

// is there a `rut.jsonc` between the file's dir and the folder root
// (inclusive of the file's dir, exclusive of the folder)? Then the
// file belongs to a NESTED package — its own manifest walk owns its
// mod paths, the folder stamp would lie
function nestedManifest(folderFs: string, fileFs: string): boolean {
  let dir = path.dirname(fileFs);
  while (dir.length > folderFs.length) {
    if (fs.existsSync(path.join(dir, MANIFEST_NAME))) {
      return true;
    }
    const parent = path.dirname(dir);
    if (parent === dir) {
      break;
    }
    dir = parent;
  }
  return false;
}

// a package file's dir relative to the manifest, minus the leading
// `src` component, is its mod path (`src` → `''` the root,
// `src/layout` → `layout`) — the package's module tree lives under its
// `src/` shell. Anything outside the shell (the old
// root-beside-manifest spelling included) is not a module file: `null`.
function stripSrcShell(rel: string): string | null {
  const norm = rel.split(path.sep).join('/');
  if (norm === 'src') {
    return '';
  }
  if (norm.startsWith('src/')) {
    return norm.slice('src/'.length);
  }
  return null;
}

async function indexWorkspace(rut: RutWasm): Promise<void> {
  const files = await vscode.workspace.findFiles(
    '**/*.rut',
    '{**/target/**,**/node_modules/**,**/.git/**,**/.*/**}',
    MAX_WORKSPACE_FILES
  );
  const dec = new TextDecoder();
  const pkgs = new Map<string, string | null>();
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    pkgs.set(folder.uri.fsPath, await folderPkgName(rut, folder.uri));
  }
  // the BFS frontier: stamped files whose `mod NAME;` edges may carry
  // single-file children (`NAME.rut` beside the declaring file)
  const frontier: Array<{
    folder: vscode.Uri;
    dir: vscode.Uri;
    pkg: string;
    modPath: string;
    decls: Array<{ name: string; vis: 'pub' | 'mod' }>;
  }> = [];
  for (const file of files) {
    const bytes = await vscode.workspace.fs.readFile(file);
    const folder = vscode.workspace.getWorkspaceFolder(file)?.uri;
    const pkg = folder ? pkgs.get(folder.fsPath) ?? null : null;
    // a `mod.rut` under a folder-level package stamps its mod path
    // (the dir relative to the folder, minus the `src/` shell —
    // `src/mod.rut` is the root `''`) — the position-path tier's data.
    // A file outside the shell keeps today's plain shape (the dep walk
    // re-stamps the deps it mounts anyway)
    if (
      pkg &&
      folder &&
      path.basename(file.fsPath) === 'mod.rut' &&
      !nestedManifest(folder.fsPath, file.fsPath)
    ) {
      const rel = path.relative(folder.fsPath, path.dirname(file.fsPath));
      const modPath = stripSrcShell(rel);
      if (modPath === null) {
        rut.addDef(file.toString(), dec.decode(bytes));
        continue;
      }
      const stamped = rut.addDefMod(file.toString(), pkg, modPath, dec.decode(bytes));
      if (!isEnvelope(stamped) && stamped.decls.length > 0) {
        frontier.push({
          folder,
          dir: vscode.Uri.file(path.dirname(file.fsPath)),
          pkg,
          modPath,
          decls: stamped.decls
        });
      }
      continue;
    }
    rut.addDef(file.toString(), dec.decode(bytes));
  }
  // the declared children, breadth-first: each stamped edge probes
  // `NAME.rut` beside the declaring file (BOTH spellings is the
  // loader's loud ambiguity — hinted once). A child stamps at (pkg,
  // child path) and its own decls enqueue — declarations are the
  // graph, bounded by the dep walk's file budget.
  let budget = MAX_DEP_FILES;
  while (frontier.length > 0 && budget > 0) {
    const { folder, dir, pkg, modPath, decls } = frontier.shift()!;
    for (const decl of decls) {
      if (budget <= 0) {
        break;
      }
      const dirChild = vscode.Uri.joinPath(dir, decl.name, 'mod.rut');
      const fileChild = vscode.Uri.joinPath(dir, `${decl.name}.rut`);
      const hasFile = fs.existsSync(fileChild.fsPath);
      const hasDir = fs.existsSync(dirChild.fsPath);
      if (hasFile && hasDir) {
        hintOnce(
          `mods:${fileChild.fsPath}`,
          `rut: ${pkg} — both ${decl.name}/mod.rut and ${decl.name}.rut exist; keep one`,
          true
        );
        continue;
      }
      if (!hasFile && !hasDir) {
        continue; // nothing under either spelling — nothing to stamp here
      }
      const childUri = hasFile ? fileChild : dirChild;
      if (nestedManifest(folder.fsPath, childUri.fsPath)) {
        continue; // a nested package's own walk owns it
      }
      const childPath = modPath === '' ? decl.name : `${modPath}/${decl.name}`;
      if (!hasFile) {
        // the directory form is the path-shape stamp's business —
        // EXCEPT under a file-form parent, whose child's bare
        // relative dir (minus the `src/` shell) is not its mod path;
        // only that correction spends the budget here
        const bareRel = path.relative(folder.fsPath, path.dirname(childUri.fsPath));
        if (stripSrcShell(bareRel) === childPath) {
          continue;
        }
      }
      const bytes = await vscode.workspace.fs.readFile(childUri);
      // addDefMod replaces per origin — a dir-form correction overrides
      // the path-shape stamp's bare-relative answer
      const stamped = rut.addDefMod(childUri.toString(), pkg, childPath, dec.decode(bytes));
      budget--;
      if (!isEnvelope(stamped) && stamped.decls.length > 0) {
        frontier.push({
          folder,
          dir: hasFile ? dir : vscode.Uri.joinPath(dir, decl.name),
          pkg,
          modPath: childPath,
          decls: stamped.decls
        });
      }
    }
  }
}

// ---- shared helpers ----

// the module's document key — the same string the Rust side stores
function analyzeDoc(rut: RutWasm, doc: vscode.TextDocument): Analysis {
  return rut.analyze(doc.uri.toString(), doc.getText());
}

// ---- LSP (JSON) -> vscode mapping ----
// LSP protocol numbers are 1-based; the vscode enums are 0-based, so the
// mapping is `- 1` across the board (severity, symbol kind, completion
// kind).

// a definition target -> a vscode Location. A target with a scheme
// parses as-is; a repo-relative path (the embedded std surface's true
// source, e.g. `rut/pouch/src/mod.rut`) resolves against the workspace
// folders — a workspace that is the rut repo gets a real jump into the
// stdlib. Nowhere to resolve: dropped (never a wrong jump). A FULLY
// dropped answer — the wasm resolved the symbol but every target is
// unopenable (a url dep not extracted yet, mostly) — hints once per
// document instead of a silent dead ctrl+click; an empty input stays
// silent (an honest miss — nothing resolved at all).
function resolveLocations(rut: RutWasm, locs: LspLocation[], srcUri: string): vscode.Location[] {
  const out: vscode.Location[] = [];
  for (const l of locs) {
    const uri = toTargetUri(l.uri);
    if (uri) {
      out.push(new vscode.Location(uri, toRange(l.range)));
    }
  }
  if (locs.length > 0 && out.length === 0) {
    hintOnce(`def-dropped:${srcUri}`, defDropHint(rut, locs), false);
  }
  return out;
}

// the drop hint names the LIKELIEST cause, not just the generic one.
// A relative `rut/…` target is the embedded std surface answering for a
// dep that never mounted; when the workspace's cache carries source
// zips, `rut install` already ran and reloading cannot help — the
// definition lives in the rut repo sources, so the hint says so.
function defDropHint(_rut: RutWasm, locs: LspLocation[]): string {
  if (locs.every((l) => l.uri.startsWith('rut/')) && workspaceHasZipCache()) {
    return 'rut: definition target not openable — the resolved definition lives in the rut ' +
      'repo sources, which are not in this workspace; reload the window if the dep was just installed';
  }
  return 'rut: definition target not openable — a dep may not be extracted yet; ' +
    'run `rut install`, then reload the window';
}

// any workspace folder's `.rut/cache/` holding a `.zip` entry — proof
// the source-zip transport is in play here (`rut install` fetched it)
function workspaceHasZipCache(): boolean {
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    const cacheDir = path.join(folder.uri.fsPath, '.rut', 'cache');
    try {
      for (const name of fs.readdirSync(cacheDir)) {
        if (name.endsWith('.zip')) {
          return true;
        }
      }
    } catch {
      // no cache dir (or unreadable) in this folder — the next one decides
    }
  }
  return false;
}

function toTargetUri(raw: string): vscode.Uri | undefined {
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(raw)) {
    return vscode.Uri.parse(raw);
  }
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    const p = path.join(folder.uri.fsPath, raw);
    if (fs.existsSync(p)) {
      return vscode.Uri.file(p);
    }
  }
  return undefined;
}

function toRange(r: LspRange): vscode.Range {
  return new vscode.Range(r.start.line, r.start.character, r.end.line, r.end.character);
}

function toDiagnostic(d: LspDiag): vscode.Diagnostic {
  const diag = new vscode.Diagnostic(
    toRange(d.range),
    d.message,
    ((d.severity ?? 1) - 1) as vscode.DiagnosticSeverity
  );
  if (d.source) {
    diag.source = d.source; // `rut` — set Rust-side (the wire test greps it)
  }
  if (d.relatedInformation) {
    diag.relatedInformation = d.relatedInformation.map(
      (r) =>
        new vscode.DiagnosticRelatedInformation(
          new vscode.Location(vscode.Uri.parse(r.location.uri), toRange(r.location.range)),
          r.message
        )
    );
  }
  return diag;
}

function toSymbol(s: LspSymbol): vscode.DocumentSymbol {
  const sym = new vscode.DocumentSymbol(
    s.name,
    s.detail ?? '',
    (s.kind - 1) as vscode.SymbolKind,
    toRange(s.range),
    toRange(s.selectionRange)
  );
  if (s.children) {
    sym.children = s.children.map(toSymbol);
  }
  return sym;
}

function toCompletionItem(c: LspCompletionItem): vscode.CompletionItem {
  const item = new vscode.CompletionItem(c.label, ((c.kind ?? 1) - 1) as vscode.CompletionItemKind);
  if (c.detail) {
    item.detail = c.detail;
  }
  if (c.documentation) {
    item.documentation = new vscode.MarkdownString(c.documentation.value);
  }
  // the auto-import tier: sorts after the locals, and accepting the
  // item inserts `use mod::Name;` as a fresh line (the engine computed
  // both over the normalized source — the extension only maps them)
  if (c.sortText) {
    item.sortText = c.sortText;
  }
  if (c.additionalTextEdits) {
    item.additionalTextEdits = c.additionalTextEdits.map((e) => new vscode.TextEdit(toRange(e.range), e.newText));
  }
  return item;
}

// LSP and vscode share the inlay-kind numbering (1 Type, 2 Parameter)
// — passed through as-is, unlike the 1-based-vs-0-based enums above.
function toInlayHint(h: LspInlayHint): vscode.InlayHint {
  const hint = new vscode.InlayHint(
    new vscode.Position(h.position.line, h.position.character),
    h.label,
    h.kind as vscode.InlayHintKind
  );
  if (h.tooltip) {
    hint.tooltip = new vscode.MarkdownString(
      typeof h.tooltip === 'string' ? h.tooltip : h.tooltip.value
    );
  }
  return hint;
}

function toSignatureHelp(h: LspSignatureHelp): vscode.SignatureHelp {
  const help = new vscode.SignatureHelp();
  help.signatures = h.signatures.map((s) => {
    const info = new vscode.SignatureInformation(
      s.label,
      s.documentation ? new vscode.MarkdownString(s.documentation.value) : undefined
    );
    info.parameters = (s.parameters ?? []).map((p) => new vscode.ParameterInformation(p.label));
    return info;
  });
  help.activeSignature = h.activeSignature ?? 0;
  if (h.activeParameter !== undefined) {
    help.activeParameter = h.activeParameter;
  }
  return help;
}
