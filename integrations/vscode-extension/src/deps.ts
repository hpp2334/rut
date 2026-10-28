// deps: the manifest walk.
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

import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { DepRow, isEnvelope, RutLsp, SrczipExtracted } from './wasm';
import {
  ANALYSIS_DEBOUNCE_MS,
  MANIFEST_NAME,
  MAX_DEP_DEPTH,
  MAX_DEP_FILES,
  MAX_FETCH_BYTES,
} from './constants';
import {
  atomicCacheWrite,
  fileHasContent,
  hintOnce,
  readFileNullable,
  sha256Hex,
} from './util';
import { indexWorkspace } from './workspace';

interface WalkCtx {
  rut: RutLsp;
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

// the dep walk is debounced like the analysis (watchers fire in
// bursts), serialized (a walk in flight outlives its trigger), and
// re-run once when triggers arrived mid-walk
let walkTimer: ReturnType<typeof setTimeout> | undefined;
let walking = false;
let walkRerun = false;

export function scheduleDepWalk(rut: RutLsp): void {
  if (walkTimer !== undefined) {
    clearTimeout(walkTimer);
  }
  walkTimer = setTimeout(() => {
    walkTimer = undefined;
    void runDepWalk(rut);
  }, ANALYSIS_DEBOUNCE_MS);
}

async function runDepWalk(rut: RutLsp): Promise<void> {
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
export function watchDeps(context: vscode.ExtensionContext, rut: RutLsp): void {
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

async function walkDeps(rut: RutLsp): Promise<void> {
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
