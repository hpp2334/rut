// rut-vscode — runs the language core as an in-process wasm module
// (rut-lsp-wasm raw ABI, RFC 0041 §2): no server process, no per-platform
// binaries — one .wasm serves every platform. Providers register directly
// against vscode.languages; the same pure queries serve the native
// `rut-lsp` stdio server for editors that speak LSP. Without
// bin/rut-lsp.wasm the TextMate grammar still colors the basics and an
// info message points at `npm run build:wasm`.
//
// Layout: `constants` (the budgets) → `util` (byte reads, the cache
// write, the hint gate) → `mapping` (LSP JSON → vscode values),
// `providers` (the nine language features), `workspace` (the fs scan),
// `deps` (the manifest walk) — this file is the lifecycle that wires
// them: one long-lived `RutLsp` instance, loaded once, shared by every
// closure below.

import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { RutLsp } from './wasm';
import { ANALYSIS_DEBOUNCE_MS } from './constants';
import { analyzeDoc } from './util';
import { toDiagnostic } from './mapping';
import {
  registerCompletion,
  registerDefinition,
  registerDocumentSymbols,
  registerHover,
  registerInlayHints,
  registerReferences,
  registerSemanticTokens,
  registerSignatureHelp,
  registerTypeDefinition,
} from './providers';
import { scheduleDepWalk, watchDeps } from './deps';
import { indexWorkspace } from './workspace';

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
  const bytes = await fs.promises.readFile(wasmPath);
  // the bytes are the shipped artifact — an invalid/missing file fails
  // init LOUDLY inside the binding, never a silent half-working state
  const rut = await RutLsp.load(bytes);
  registerAll(context, rut);
}

function registerAll(context: vscode.ExtensionContext, rut: RutLsp): void {
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
