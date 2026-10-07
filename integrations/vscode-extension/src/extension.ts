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
import { Analysis, DepRow, isEnvelope, LspCompletionItem, LspDiag, LspInlayHint, LspLocation, LspRange, LspSignatureHelp, LspSymbol, RutWasm } from './wasm';

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

  // workspace index off the hot activation path — hover/completion may
  // briefly resolve against the std surface + open docs only. The dep
  // walk rides after it: the root `rut.jsonc`'s rows mount the deps
  // (path dirs, pinned url bundles) into the same index.
  watchDeps(context, rut);
  void indexWorkspace(rut).then(() => scheduleDepWalk(rut));
}

// ---- deps: the manifest walk ----
//
// After the workspace scan each workspace folder's root `rut.jsonc` is
// handed to the wasm (`rut_parse_manifest` — JS moves bytes, the wasm
// parses; law: no JS-side rut parsing). Path deps resolve against the
// manifest dir — OUTSIDE the workspace folder is the point — and
// recurse cycle-safe under a depth/file budget. Url deps ride the
// shared `.rut/cache/` (the Rust-computed cache_path, the same dir the
// CLI fills): cache-first, a hit verified against the sha256 pin (a
// poisoned entry evicts and re-fetches — the CLI's healing law), a
// miss GETs, pins the bytes BEFORE they land anywhere, and writes the
// cache atomically (tmp + rename). No pin, no bytes — one error hint.
// Every failure is ONE hint per dep naming the CLI alternative: never
// spam, never silent wrong results.

interface WalkCtx {
  rut: RutWasm;
  /// the anchoring workspace folder — where `.rut/cache/` lives
  folder: vscode.Uri;
  /// resolved manifest dirs — the cycle break
  seen: Set<string>;
  /// remaining dep-file budget
  files: number;
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

// `**/rut.jsonc` + `.rut/cache/**` re-run the walk; a workspace-folder
// change restarts the whole index (watchers re-arm over the new folder
// set, the scan + walk re-run — re-indexing is replace-by-origin, so
// this is idempotent)
function watchDeps(context: vscode.ExtensionContext, rut: RutWasm): void {
  const watchers: vscode.Disposable[] = [];
  const watchFolder = (folder: vscode.WorkspaceFolder): void => {
    for (const pattern of [`**/${MANIFEST_NAME}`, '.rut/cache/**']) {
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
    const ctx: WalkCtx = { rut, folder: folder.uri, seen: new Set(), files: MAX_DEP_FILES };
    await walkManifest(ctx, vscode.Uri.joinPath(folder.uri, MANIFEST_NAME), 1);
  }
}

// one manifest: its own entry sources index under the module name (the
// uri is the file's real on-disk URI so F12 jumps land), then its dep
// rows walk — `deps` at any depth, `dev-deps` only at the root (the
// loader's law: a dep's dev table never enters a consumer's world),
// `peer-deps` only when locally resolvable (the consumer supplies
// them; an unresolvable peer is the normal state, not a failure).
// Namespaced/const-carrying path deps (calc's `Math`) index their
// surface only — namespace/consts have no voice in `rut_add_def_named`;
// a dep needing them mounts through a bundle.
async function walkManifest(ctx: WalkCtx, manifestUri: vscode.Uri, depth: number): Promise<void> {
  if (depth > MAX_DEP_DEPTH || ctx.files <= 0) {
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
  for (const entry of table.entries) {
    if (ctx.files <= 0) {
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
  const rows: Array<{ row: DepRow; peer: boolean }> = [
    ...table.deps.map((row) => ({ row, peer: false })),
    ...(depth === 1 ? table.dev_deps : []).map((row) => ({ row, peer: false })),
    ...table.peer_deps.map((row) => ({ row, peer: true })),
  ];
  for (const { row, peer } of rows) {
    if (ctx.files <= 0) {
      return;
    }
    if (row.source.kind === 'url') {
      await mountUrlDep(ctx, row);
      continue;
    }
    const depDir = path.resolve(dirKey, row.source.dir);
    const depManifestUri = vscode.Uri.file(path.join(depDir, MANIFEST_NAME));
    if (!fs.existsSync(depManifestUri.fsPath)) {
      if (!peer) {
        hintOnce(
          `path:${dirKey}:${row.name}`,
          `rut: dep '${row.name}' — ${row.source.dir} has no ${MANIFEST_NAME}; ` +
            `a path dep is a module directory (one manifest per module)`,
          true
        );
      }
      continue;
    }
    // the optional peer's integration group (`lib` — an impl-only
    // source in the OWNER's directory) is the owner's surface gated on
    // the peer's presence: index it under the owner's module name,
    // only when the peer resolved
    if (peer && row.optional && row.lib !== null) {
      const libUri = vscode.Uri.file(path.resolve(dirKey, row.lib));
      const libSrc = await readFileNullable(libUri);
      if (libSrc !== null && ctx.files > 0) {
        ctx.files--;
        ctx.rut.addDefNamed(libUri.toString(), module, new TextDecoder().decode(libSrc));
      }
    }
    await walkManifest(ctx, depManifestUri, depth + 1);
  }
}

// one url row — the mount door. `sha256`/`cache_path` are Rust-computed
// (the row's envelope carries them; JS never hashes URLs).
async function mountUrlDep(ctx: WalkCtx, row: DepRow): Promise<void> {
  const source = row.source;
  if (source.kind !== 'url') {
    return;
  }
  const { url, sha256, cache_path } = source;
  if (sha256 === null) {
    hintOnce(
      `unpinned:${url}`,
      `rut: dep '${row.name}' has no sha256 pin — the editor never mounts unpinned url ` +
        `bytes; pin the bundle's hash beside its url in ${MANIFEST_NAME}`,
      true
    );
    return;
  }
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
  const bundle = ctx.rut.addBundle(bytes);
  if (isEnvelope(bundle)) {
    hintOnce(
      `bundle:${url}`,
      `rut: dep '${row.name}' — the bundle did not mount: ${bundle.error}`,
      true
    );
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
  const tmp = target.with({ path: target.path.replace(/\.rutbundle$/, '.part') });
  await vscode.workspace.fs.writeFile(tmp, bytes);
  await vscode.workspace.fs.rename(tmp, target, { overwrite: true });
}

// a missing or unreadable file is a null, not a failure — callers
// decide whether that's a miss (cache), a skip (peer), or a hint (dep)
async function readFileNullable(file: vscode.Uri): Promise<Uint8Array | null> {
  try {
    return await vscode.workspace.fs.readFile(file);
  } catch {
    return null;
  }
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
        return rut.complete(doc.uri.toString(), position.line, position.character).map(toCompletionItem);
      },
    },
    '.' // trigger: member completion after a receiver dot
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
        rut.definition(doc.uri.toString(), position.line, position.character)
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
        rut.typeDefinition(doc.uri.toString(), position.line, position.character)
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
        rut.references(doc.uri.toString(), position.line, position.character, context.includeDeclaration)
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

async function indexWorkspace(rut: RutWasm): Promise<void> {
  const files = await vscode.workspace.findFiles(
    '**/*.rut',
    '{**/target/**,**/node_modules/**,**/.git/**,**/.*/**}',
    MAX_WORKSPACE_FILES
  );
  const dec = new TextDecoder();
  for (const file of files) {
    const bytes = await vscode.workspace.fs.readFile(file);
    rut.addDef(file.toString(), dec.decode(bytes));
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
// source, e.g. `rut/pouch/pouch.rut`) resolves against the workspace
// folders — a workspace that is the rut repo gets a real jump into the
// stdlib. Nowhere to resolve: dropped (a clean miss, never a wrong jump).
function resolveLocations(locs: LspLocation[]): vscode.Location[] {
  const out: vscode.Location[] = [];
  for (const l of locs) {
    const uri = toTargetUri(l.uri);
    if (uri) {
      out.push(new vscode.Location(uri, toRange(l.range)));
    }
  }
  return out;
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
