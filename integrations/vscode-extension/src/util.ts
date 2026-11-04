// shared helpers — byte reads, the once-per-reason hint gate, and the
// document analyze entry every provider lane rides. The dep-walk
// helpers (the pin hash, the cache write) are GONE: the walk lives in
// the crate and the extension writes nothing.

import * as vscode from 'vscode';
import type { Analysis, RutLsp } from './wasm';

// a missing or unreadable file is a null, not a failure — callers
// decide whether that's a miss (scan) or a hint (dep)
export async function readFileNullable(file: vscode.Uri): Promise<Uint8Array | null> {
    try {
        return await vscode.workspace.fs.readFile(file);
    } catch {
        return null;
    }
}

// hints are once-per-reason per session: a re-walk fires dozens of
// times while the world is cold — the first hint names the fix, the
// rest would be spam
const hintsShown = new Set<string>();

export function hintOnce(key: string, message: string, error: boolean): void {
    if (hintsShown.has(key)) {
        return;
    }
    hintsShown.add(key);
    const shown = error
        ? vscode.window.showErrorMessage(message)
        : vscode.window.showInformationMessage(message);
    void Promise.resolve(shown).catch(() => undefined);
}

// the module's document key — the same string the Rust side stores
export async function analyzeDoc(rut: RutLsp, doc: vscode.TextDocument): Promise<Analysis> {
    return rut.analyze(doc.uri.toString(), doc.getText());
}
