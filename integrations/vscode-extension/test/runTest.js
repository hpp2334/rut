// In-Extension-Host test — run via:
//   Code.exe --extensionDevelopmentPath=<ext> --extensionTestsPath=<this file>
// Exports run(); VS Code calls it after activation, exit code reflects the result.
'use strict';

const assert = require('node:assert');
const { join } = require('node:path');

const EXT_ID = 'rut.rut-vscode';
// the symbol fixture the test asserts on (relative to the extension, so
// the test runs on every machine)
const FIXTURE = join(__dirname, 'fixtures', 'symbols.rut');

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

  log('ALL CHECKS PASSED');
}

module.exports = { run };
