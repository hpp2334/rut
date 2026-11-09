// the nine language-feature providers — each a thin body over the
// wasm's query: pull the answer, map it onto vscode values (mapping.ts),
// hand it back. Every binding call is an async RPC bridge (the actor
// seam), so every provider is async — VS Code's ProviderResult accepts
// Thenables. VS Code owns scheduling; the debounce lives in
// extension.ts's diagnostics lane.

import * as vscode from 'vscode';
import { SELECTOR } from './constants';
import {
    resolveLocations,
    toCompletionItem,
    toInlayHint,
    toRange,
    toSignatureHelp,
    toSymbol,
} from './mapping';
import { analyzeDoc } from './util';
import { decodeTokens, type RutLsp } from './wasm';

export async function registerSemanticTokens(rut: RutLsp): Promise<vscode.Disposable> {
    const legend = new vscode.SemanticTokensLegend(await rut.legend(), []);
    return vscode.languages.registerDocumentSemanticTokensProvider(
        SELECTOR,
        {
            async provideDocumentSemanticTokens(
                doc: vscode.TextDocument,
            ): Promise<vscode.SemanticTokens> {
                const data = (await analyzeDoc(rut, doc)).tokens.data;
                const builder = new vscode.SemanticTokensBuilder(legend);
                // decode the LSP delta encoding into the builder's absolute slots
                // (one decoder for every host — the binding owns the wire)
                for (const token of decodeTokens(data)) {
                    builder.push(token.line, token.start, token.length, token.type);
                }
                return builder.build();
            },
        },
        legend,
    );
}

export function registerDocumentSymbols(rut: RutLsp): vscode.Disposable {
    return vscode.languages.registerDocumentSymbolProvider(SELECTOR, {
        async provideDocumentSymbols(doc: vscode.TextDocument): Promise<vscode.DocumentSymbol[]> {
            return (await analyzeDoc(rut, doc)).symbols.map(toSymbol);
        },
    });
}

export function registerHover(rut: RutLsp): vscode.Disposable {
    return vscode.languages.registerHoverProvider(SELECTOR, {
        async provideHover(
            doc: vscode.TextDocument,
            position: vscode.Position,
        ): Promise<vscode.Hover | undefined> {
            const h = await rut.hover(doc.uri.toString(), position.line, position.character);
            if (!h) {
                return undefined;
            }
            return new vscode.Hover(
                new vscode.MarkdownString(h.contents.value),
                h.range ? toRange(h.range) : undefined,
            );
        },
    });
}

export function registerCompletion(rut: RutLsp): vscode.Disposable {
    return vscode.languages.registerCompletionItemProvider(
        SELECTOR,
        {
            async provideCompletionItems(
                doc: vscode.TextDocument,
                position: vscode.Position,
            ): Promise<vscode.CompletionItem[]> {
                // the CURRENT text rides the request (the doc-sync entry):
                // completion fires on the keystroke while the analyze lane
                // debounces ~200 ms — the stored doc may still be short the `.`
                // this very request is about
                return (
                    await rut.complete(
                        doc.uri.toString(),
                        position.line,
                        position.character,
                        doc.getText(),
                    )
                ).map(toCompletionItem);
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
// real URIs (the core prelude's jumps carry repo-relative paths into
// the true rut/ sources; dep-lane targets land in the store's
// mounted files)
export function registerDefinition(rut: RutLsp): vscode.Disposable {
    return vscode.languages.registerDefinitionProvider(SELECTOR, {
        async provideDefinition(
            doc: vscode.TextDocument,
            position: vscode.Position,
        ): Promise<vscode.Location[]> {
            return resolveLocations(
                await rut.definition(doc.uri.toString(), position.line, position.character),
                doc.uri.toString(),
            );
        },
    });
}

export function registerTypeDefinition(rut: RutLsp): vscode.Disposable {
    return vscode.languages.registerTypeDefinitionProvider(SELECTOR, {
        async provideTypeDefinition(
            doc: vscode.TextDocument,
            position: vscode.Position,
        ): Promise<vscode.Location[]> {
            return resolveLocations(
                await rut.typeDefinition(doc.uri.toString(), position.line, position.character),
                doc.uri.toString(),
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
        async provideInlayHints(
            doc: vscode.TextDocument,
            range: vscode.Range,
        ): Promise<vscode.InlayHint[]> {
            return (await rut.inlayHint(doc.uri.toString(), range.start, range.end)).map(
                toInlayHint,
            );
        },
    });
}

// Shift+F12 / Find All References — the wasm references query (the
// definition index read backwards); the LSP includeDeclaration toggle
// rides the vscode ReferenceContext through
export function registerReferences(rut: RutLsp): vscode.Disposable {
    return vscode.languages.registerReferenceProvider(SELECTOR, {
        async provideReferences(
            doc: vscode.TextDocument,
            position: vscode.Position,
            context: vscode.ReferenceContext,
        ): Promise<vscode.Location[]> {
            return resolveLocations(
                await rut.references(
                    doc.uri.toString(),
                    position.line,
                    position.character,
                    context.includeDeclaration,
                ),
                doc.uri.toString(),
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
            async provideSignatureHelp(
                doc: vscode.TextDocument,
                position: vscode.Position,
            ): Promise<vscode.SignatureHelp | undefined> {
                const h = await rut.signatureHelp(
                    doc.uri.toString(),
                    position.line,
                    position.character,
                );
                return h ? toSignatureHelp(h) : undefined;
            },
        },
        '(',
        ',',
    );
}
