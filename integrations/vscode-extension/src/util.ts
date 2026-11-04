// shared helpers — byte reads, the cache write, the pin hash, the
// once-per-reason hint gate, and the document analyze entry every
// provider lane rides

import * as crypto from 'node:crypto';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import type { Analysis, RutLsp } from './wasm';

// the sha256 pin check — Node's webcrypto, hex-lowercase like the
// manifest's normalized pin
export async function sha256Hex(bytes: Uint8Array): Promise<string> {
    const digest = await crypto.webcrypto.subtle.digest('SHA-256', bytes);
    return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

// the CLI's atomic entry write: tmp beside the target, then rename —
// a reader never sees a partial entry
export async function atomicCacheWrite(target: vscode.Uri, bytes: Uint8Array): Promise<void> {
    await vscode.workspace.fs.createDirectory(vscode.Uri.joinPath(target, '..'));
    const tmp = target.with({ path: `${target.path}.part` });
    await vscode.workspace.fs.writeFile(tmp, bytes);
    await vscode.workspace.fs.rename(tmp, target, { overwrite: true });
}

// a missing or unreadable file is a null, not a failure — callers
// decide whether that's a miss (cache) or a hint (dep)
export async function readFileNullable(file: vscode.Uri): Promise<Uint8Array | null> {
    try {
        return await vscode.workspace.fs.readFile(file);
    } catch {
        return null;
    }
}

// does the file exist with EXACTLY this text? — the extracted-tree
// writes ride it: an unchanged tree never rewrites, so the watchers'
// re-walk settles (the first mount writes, a drifted file heals)
export async function fileHasContent(file: vscode.Uri, src: string): Promise<boolean> {
    const bytes = await readFileNullable(file);
    return bytes !== null && new TextDecoder().decode(bytes) === src;
}

// hints are once-per-reason per session: a watcher re-walk fires
// dozens of times while the network is down — the first hint names the
// fix, the rest would be spam
const depHintsShown = new Set<string>();

export function hintOnce(key: string, message: string, error: boolean): void {
    if (depHintsShown.has(key)) {
        return;
    }
    depHintsShown.add(key);
    const shown = error
        ? vscode.window.showErrorMessage(message)
        : vscode.window.showInformationMessage(message);
    void Promise.resolve(shown).catch(() => undefined);
}

// any workspace folder's `.rut/cache/` holding a `.zip` entry — proof
// the source-zip transport is in play here (`rut install` fetched it)
export function workspaceHasZipCache(): boolean {
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
        const cacheDir = path.join(folder.uri.fsPath, '.rut', 'cache');
        try {
            for (const name of fs.readdirSync(cacheDir)) {
                if (name.endsWith('.zip')) {
                    return true;
                }
            }
        } catch {
            // no cache dir (or unreadable) in this folder — the next one decides
        }
    }
    return false;
}

// the module's document key — the same string the Rust side stores
export async function analyzeDoc(rut: RutLsp, doc: vscode.TextDocument): Promise<Analysis> {
    return rut.analyze(doc.uri.toString(), doc.getText());
}
