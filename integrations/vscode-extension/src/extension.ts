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
import { Analysis, LspCompletionItem, LspDiag, LspInlayHint, LspLocation, LspRange, LspSignatureHelp, LspSymbol, RutWasm } from './wasm';

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
    registerCompletion(rut),
    registerDefinition(rut),
    registerTypeDefinition(rut),
    registerReferences(rut),
    registerInlayHints(rut),
    registerSignatureHelp(rut)
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
