// the nine language-feature providers — each a thin body over the
// wasm's query: pull the answer, map it onto vscode values (mapping.ts),
// hand it back. VS Code owns scheduling; the debounce lives in
// extension.ts's diagnostics lane.

import * as vscode from 'vscode';
import { decodeTokens, RutLsp } from './wasm';
import { SELECTOR } from './constants';
import { analyzeDoc } from './util';
import {
  resolveLocations,
  toCompletionItem,
  toInlayHint,
  toRange,
  toSignatureHelp,
  toSymbol,
} from './mapping';

export function registerSemanticTokens(rut: RutLsp): vscode.Disposable {
  const legend = new vscode.SemanticTokensLegend(rut.legend(), []);
  return vscode.languages.registerDocumentSemanticTokensProvider(
    SELECTOR,
    {
      provideDocumentSemanticTokens(doc: vscode.TextDocument): vscode.SemanticTokens {
        const data = analyzeDoc(rut, doc).tokens.data;
        const builder = new vscode.SemanticTokensBuilder(legend);
        // decode the LSP delta encoding into the builder's absolute slots
        // (one decoder for every host — the binding owns the wire)
        for (const token of decodeTokens(data)) {
          builder.push(token.line, token.start, token.length, token.type);
        }
        return builder.build();
      },
    },
    legend
  );
}

export function registerDocumentSymbols(rut: RutLsp): vscode.Disposable {
  return vscode.languages.registerDocumentSymbolProvider(SELECTOR, {
    provideDocumentSymbols(doc: vscode.TextDocument): vscode.DocumentSymbol[] {
      return analyzeDoc(rut, doc).symbols.map(toSymbol);
    },
  });
}

export function registerHover(rut: RutLsp): vscode.Disposable {
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

export function registerCompletion(rut: RutLsp): vscode.Disposable {
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
export function registerDefinition(rut: RutLsp): vscode.Disposable {
  return vscode.languages.registerDefinitionProvider(SELECTOR, {
    provideDefinition(
      doc: vscode.TextDocument,
      position: vscode.Position
    ): vscode.Location[] {
      return resolveLocations(
        rut.definition(doc.uri.toString(), position.line, position.character),
        doc.uri.toString()
      );
    },
  });
}

export function registerTypeDefinition(rut: RutLsp): vscode.Disposable {
  return vscode.languages.registerTypeDefinitionProvider(SELECTOR, {
    provideTypeDefinition(
      doc: vscode.TextDocument,
      position: vscode.Position
    ): vscode.Location[] {
      return resolveLocations(
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
export function registerInlayHints(rut: RutLsp): vscode.Disposable {
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
export function registerReferences(rut: RutLsp): vscode.Disposable {
  return vscode.languages.registerReferenceProvider(SELECTOR, {
    provideReferences(
      doc: vscode.TextDocument,
      position: vscode.Position,
      context: vscode.ReferenceContext
    ): vscode.Location[] {
      return resolveLocations(
        rut.references(doc.uri.toString(), position.line, position.character, context.includeDeclaration),
        doc.uri.toString()
      );
    },
  });
}

// signature help (`(` / `,` triggers) — the callee resolves through the
// same machinery the param-name hints ride; a mismatch shows nothing
export function registerSignatureHelp(rut: RutLsp): vscode.Disposable {
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
