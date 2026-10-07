// In-Extension-Host test — run via:
//   Code.exe --extensionDevelopmentPath=<ext> --extensionTestsPath=<this file>
// Exports run(); VS Code calls it after activation, exit code reflects the result.
'use strict';

const assert = require('node:assert');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const { join } = require('node:path');

const EXT_ID = 'rut.rut-vscode';
// the symbol fixture the test asserts on (relative to the extension, so
// the test runs on every machine) — the fixture's own module dir, its
// entry source is what the symbol assertions read
const FIXTURE = join(__dirname, 'fixtures', 'symbols', 'symbols.rut');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function poll(what, fn, tries = 100, gap = 100) {
  for (let i = 0; i < tries; i++) {
    const v = await fn();
    if (v) return v;
    await sleep(gap);
  }
  throw new Error(`timed out waiting for ${what}`);
}

async function run() {
  const vscode = require('vscode');
  const log = (...a) => console.log('[rut-test]', ...a);

  // 1. the extension activates (opening a .rut doc fires onLanguage:rut)
  const uri = vscode.Uri.file(FIXTURE);
  const doc = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(doc);
  const ext = vscode.extensions.getExtension(EXT_ID);
  assert.ok(ext, `extension ${EXT_ID} not found — check package.json name/publisher`);
  await poll('extension activation', () => ext.isActive);
  log('extension active');

  // 2. language registration
  assert.strictEqual(doc.languageId, 'rut', `languageId is ${doc.languageId}, not 'rut'`);
  log('languageId = rut');

  // 3. semantic tokens arrive (proves the client launched rut-lsp and it answered)
  const legend = await vscode.commands.executeCommand('vscode.provideDocumentSemanticTokensLegend', doc.uri);
  assert.ok(legend && Array.isArray(legend.tokenTypes) && legend.tokenTypes.includes('enumMember'),
    `bad legend: ${JSON.stringify(legend)}`);
  const tokens = await poll('semantic tokens', () =>
    vscode.commands.executeCommand('vscode.provideDocumentSemanticTokens', doc.uri));
  assert.ok(tokens && tokens.data && tokens.data.length >= 100, `thin token stream: ${tokens && tokens.data && tokens.data.length}`);
  log(`semantic tokens: ${tokens.data.length / 5} tokens, legend ok (${legend.tokenTypes.length} types)`);

  // 4. document symbols
  const syms = await poll('document symbols', () =>
    vscode.commands.executeCommand('vscode.executeDocumentSymbolProvider', doc.uri));
  assert.ok(Array.isArray(syms) && syms.length > 0, `no symbols: ${JSON.stringify(syms)}`);
  const names = syms.map((s) => s.name);
  for (const want of ['Color', 'Point', 'Circle', 'Drawable', 'main']) {
    assert.ok(names.includes(want), `symbol ${want} missing (got ${names.join(', ')})`);
  }
  log(`symbols: ${names.join(' | ')}`);

  // 5. diagnostics: a broken document must produce a rut-sourced error
  const broken = await vscode.workspace.openTextDocument({ language: 'rut', content: 'fn broken(: nil {\n' });
  await vscode.window.showTextDocument(broken, { preview: true });
  const diags = await poll('diagnostics', async () => {
    const d = vscode.languages.getDiagnostics(broken.uri);
    return d.length > 0 ? d : null;
  });
  assert.strictEqual(diags[0].source, 'rut', `diag source: ${JSON.stringify(diags[0])}`);
  assert.strictEqual(diags[0].severity, vscode.DiagnosticSeverity.Error);
  log(`diagnostics: "${diags[0].message.slice(0, 48)}..." (source=rut)`);

  // 6. the dep walk — the extension reads rut.jsonc
  await depWalkChecks(log);

  log('ALL CHECKS PASSED');
}

// ---- the dep walk (the manifest-deps phase) ----
//
// A fixture workspace whose path dep lives OUTSIDE the folder boundary
// (invisible to findFiles, visible only to the walk) plus a url dep
// served by a localhost fixture server — the CLI tests' offline-remote
// pattern (one payload, any path; closed = the offline simulation).
// Scenarios, in order (the pin refusal MUST precede the good mount —
// the wasm session keeps mounted indexes, so a not-mounted assertion
// is only honest before any mount):
//   (a) out-of-folder path dep completes (`use gadgets::` tier);
//   (b) a WRONG pin → the one error hint, the bundle never mounts;
//   (c) the real pin → fetched, pinned, atomically cached (the CLI's
//       `.rut/cache/<sha256(url)>.rutbundle`), mounted — the bundle's
//       own index answers go-to-definition (`bundle:` targets);
//   (d) auto-import: the bare `Wi` probe completes Widget with the
//       `use gadgets::Widget;` insert after the last leading `use`;
//   (e) server down + cache evicted → ONE info hint naming `rut fetch`,
//       no crash, the mounted dep still resolves.
async function depWalkChecks(log) {
  const EXT = join(__dirname, '..');
  const REPO = join(EXT, '..', '..');
  const FIXTURES = join(__dirname, 'fixtures', 'dep-walk');

  // the url-dep fixture: the committed pouch bundle (the offline
  // url-dep fixture the CLI tests and the wasm e2e use) + its pin
  const bundleBytes = fs.readFileSync(join(REPO, 'dist', 'std', 'pouch.rutbundle'));
  const pin = crypto.createHash('sha256').update(bundleBytes).digest('hex');
  const badPin = '0'.repeat(64);

  const server = http.createServer((_req, res) => {
    res.writeHead(200, {
      'Content-Type': 'application/octet-stream',
      'Content-Length': bundleBytes.length,
    });
    res.end(bundleBytes);
  });
  const port = await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => resolve(server.address().port));
  });
  const url = `http://127.0.0.1:${port}/pouch.rutbundle`;

  // the scratch world: ws/ becomes a workspace folder; outside/ sits
  // beside it — outside the folder boundary by construction
  const tmp = fs.mkdtempSync(join(os.tmpdir(), 'rut-depwalk-'));
  const ws = join(tmp, 'ws');
  fs.mkdirSync(ws);
  fs.cpSync(join(FIXTURES, 'outside'), join(tmp, 'outside'), { recursive: true });
  fs.copyFileSync(join(FIXTURES, 'workspace', 'app.rut'), join(ws, 'app.rut'));
  // the cache filename is sha256 of the URL STRING (Rust-computed in
  // the real walk; the same constant here)
  const cacheEntry = join(ws, '.rut', 'cache', `${crypto.createHash('sha256').update(url).digest('hex')}.rutbundle`);
  const renderManifest = (pouchRow) => {
    const template = fs.readFileSync(join(FIXTURES, 'workspace', 'rut.jsonc.template'), 'utf8');
    fs.writeFileSync(join(ws, 'rut.jsonc'), template.replaceAll('__POUCH_ROW__', pouchRow));
  };
  const badPinRow = `    "pouch": { "url": "${url}", "sha256": "${badPin}" }`;
  const goodPinRow = `    "pouch": { "url": "${url}", "sha256": "${pin}" }`;

  // capture the walk's hints (once-per-reason per session — the
  // assertions below each clear the array first)
  const messages = [];
  const realError = vscode.window.showErrorMessage;
  const realInfo = vscode.window.showInformationMessage;
  vscode.window.showErrorMessage = (m) => {
    messages.push(String(m));
    return Promise.resolve(undefined);
  };
  vscode.window.showInformationMessage = (m) => {
    messages.push(String(m));
    return Promise.resolve(undefined);
  };

  try {
    renderManifest(badPinRow);
    assert.ok(
      vscode.workspace.updateWorkspaceFolders(vscode.workspace.workspaceFolders.length, 0, {
        uri: vscode.Uri.file(ws),
      }),
      'adding the fixture workspace folder failed'
    );

    // (a) the OUT-OF-FOLDER path dep completes in the `use <name>::` tier
    const gadgetsProbe = await vscode.workspace.openTextDocument({
      language: 'rut',
      content: 'use gadgets::\nfn main() -> nil {\n}\n',
    });
    await poll('out-of-folder dep completion', async () => {
      const items = await vscode.commands.executeCommand(
        'vscode.executeCompletionItemProvider',
        gadgetsProbe.uri,
        new vscode.Position(0, 13)
      );
      const labels = items.map((i) => i.label);
      return labels.includes('Widget') && labels.includes('tag') ? labels : null;
    });
    log('dep walk: out-of-folder path dep `gadgets` completes');

    // (b) the wrong pin: the walk ran (the hint proves it — it may
    // have arrived while (a) polled, so the array is NOT cleared), the
    // error names the mismatch, and the bundle never mounts
    const pouchProbe = await vscode.workspace.openTextDocument({
      language: 'rut',
      content: 'use pouch::{ Vec };\nfn main() -> nil {\n    let v: Vec<i32> = Vec.new();\n    v.push(1);\n}\n',
    });
    const bundleTargets = async () => {
      const locs = await vscode.commands.executeCommand(
        'vscode.executeDefinitionProvider',
        pouchProbe.uri,
        new vscode.Position(0, 13) // the use-line `Vec`
      );
      return locs.filter((l) => l.uri.toString().startsWith('bundle:'));
    };
    const refusal = await poll('pin-mismatch hint', async () => {
      const m = messages.find((x) => x.includes('sha256 pin mismatch'));
      if (!m) return null;
      assert.ok(m.includes(url), `the refusal names the url: ${m}`);
      assert.ok(m.includes(badPin), `the refusal names the pin: ${m}`);
      return m;
    });
    log(`dep walk: bad pin refused — "${refusal.slice(0, 60)}..."`);
    // the negative is honest only against an ANALYZED probe: wait for
    // the std surface's own pouch answer first, then demand that the
    // bundle added none
    await poll('std pouch answers (probe analyzed)', async () =>
      (await vscode.commands.executeCommand(
        'vscode.executeDefinitionProvider',
        pouchProbe.uri,
        new vscode.Position(0, 13)
      )).length > 0
        ? true
        : null
    );
    assert.strictEqual(
      (await bundleTargets()).length, 0,
      'a bad pin must never mount the bundle'
    );

    // (c) the real pin: fetched + cached + mounted — the bundle's own
    // index answers go-to-definition beside the std surface's
    messages.length = 0;
    renderManifest(goodPinRow);
    await poll('url dep mounted (bundle: go-to-def)', async () =>
      (await bundleTargets()).length > 0 ? true : null
    );
    await poll('cache entry landed', () => (fs.existsSync(cacheEntry) ? true : null));
    log(`dep walk: pinned url dep mounted + cached (${cacheEntry.length} char path)`);

    // (d) auto-import: the bare `Wi` probe offers Widget with the
    // insert after the last leading `use`, sorted after the locals
    const autoProbe = await vscode.workspace.openTextDocument({
      language: 'rut',
      content: 'use gadgets::{ tag };\nfn main() -> nil {\n    let w = Wi;\n}\n',
    });
    const imp = await poll('auto-import item', async () => {
      const items = await vscode.commands.executeCommand(
        'vscode.executeCompletionItemProvider',
        autoProbe.uri,
        new vscode.Position(2, 14) // right after the `Wi` fragment
      );
      // the plain `class Widget` item and the auto-import item share
      // the label — the import offer is the one carrying the edit
      const item = items.find(
        (i) => i.label === 'Widget' && i.additionalTextEdits && i.additionalTextEdits.length === 1
      );
      return item || null;
    });
    const edit = imp.additionalTextEdits[0];
    assert.strictEqual(
      edit.newText, '\nuse gadgets::Widget;',
      `the insert is a fresh line naming the module: ${JSON.stringify(edit.newText)}`
    );
    assert.strictEqual(edit.range.start.line, 0, 'the insert rides after the last leading `use`');
    assert.strictEqual(edit.range.start.line, edit.range.end.line, 'the insert is one fresh line');
    assert.ok(imp.sortText && imp.sortText.startsWith('~'), `the tier sorts after locals: ${imp.sortText}`);
    log('dep walk: auto-import inserts `use gadgets::Widget;` after the leading use');

    // (e) server down + cache evicted: the walk re-runs (the cache
    // watcher fires on the delete), ONE info hint names the CLI
    // alternative, nothing crashes, the mounted dep still resolves
    messages.length = 0;
    await new Promise((resolve) => server.close(resolve));
    fs.unlinkSync(cacheEntry);
    const hint = await poll('fetch-fail hint', () =>
      messages.find((x) => x.includes('rut fetch')) || null
    );
    assert.ok(hint.includes('pouch'), `the hint names the dep: ${hint}`);
    assert.ok((await bundleTargets()).length > 0, 'the mounted dep outlives the cache eviction');
    log(`dep walk: offline + cache miss → "${hint.slice(0, 60)}..."`);
  } finally {
    vscode.window.showErrorMessage = realError;
    vscode.window.showInformationMessage = realInfo;
    server.close();
  }
}

module.exports = { run };
