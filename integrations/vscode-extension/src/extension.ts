// rut-vscode — runs the language core as an in-process wasm module
// (rut-lsp-wasm raw ABI, RFC 0041 §2): no server process, no per-platform
// binaries — one .wasm serves every platform. Providers register directly
// against vscode.languages; the same pure queries serve the native
// `rut-lsp` stdio server for editors that speak LSP. Without
// bin/rut-lsp.wasm the TextMate grammar still colors the basics and an
// info message points at `npm run build:wasm`.

import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { Analysis, LspCompletionItem, LspDiag, LspRange, LspSymbol, RutWasm } from './wasm';

const SELECTOR: vscode.DocumentSelector = { language: 'rut' };

// keystrokes coalesce: the last change inside the window wins (the
// stdio server got this from the language client's batching)
const ANALYSIS_DEBOUNCE_MS = 200;

// mirrors the native server's workspace-scan cap — an LSP is a guest,
// not an indexer daemon
const MAX_WORKSPACE_FILES = 500;

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
    registerCompletion(rut)
  );

  // workspace index off the hot activation path — hover/completion may
  // briefly resolve against the std surface + open docs only
  void indexWorkspace(rut);
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
  return item;
}
