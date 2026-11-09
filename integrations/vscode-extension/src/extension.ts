// rut-vscode — runs the language core as an in-process wasm module
// (rut-lsp-wasm raw ABI, RFC 0041 §2): no server process, no per-platform
// binaries — one .wasm serves every platform. Providers register directly
// against vscode.languages; the same pure queries serve the native
// `rut-lsp` stdio server for editors that speak LSP. Without
// bin/rut-lsp.wasm the TextMate grammar still colors the basics and an
// info message points at `npm run build:wasm`.
//
// Layout: `constants` (the budgets) → `util` (byte reads, the hint
// gate) → `mapping` (LSP JSON → vscode values), `providers` (the nine
// language features), `workspace` (the fs scan), `env` (the read-only
// host environment the crate's dep walk reads through) — this file is
// the lifecycle that wires them: one long-lived `RutLsp` instance,
// loaded once, shared by every closure below. The dep walk itself
// lives IN the crate (`resolveDeps` + the crate-shipped `depEngine`);
// the extension only injects the capability, watches, and stirs.

import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { ANALYSIS_DEBOUNCE_MS, MANIFEST_NAME } from './constants';
import { hostEnv } from './env';
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
import { analyzeDoc } from './util';
import { depEngine, RutLsp } from './wasm';
import { indexWorkspace } from './workspace';

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    const wasmPath = path.join(context.extensionPath, 'bin', 'rut-lsp.wasm');
    if (!fs.existsSync(wasmPath)) {
        vscode.window.showInformationMessage(
            'rut-lsp.wasm not found — run `npm run build:wasm` in integrations/vscode-extension ' +
                'for semantic highlighting, diagnostics, symbols, hover and completions. ' +
                'Basic grammar highlighting stays on.',
        );
        return;
    }
    const bytes = await fs.promises.readFile(wasmPath);
    // the bytes are the shipped artifact — an invalid/missing file fails
    // init LOUDLY inside the binding, never a silent half-working state
    const rut = await RutLsp.load(bytes);
    await registerAll(context, rut);
}

async function registerAll(context: vscode.ExtensionContext, rut: RutLsp): Promise<void> {
    const subscriptions = context.subscriptions;

    // ---- pushed diagnostics (the wasm twin of publish_diagnostics) ----
    const diagnostics = vscode.languages.createDiagnosticCollection('rut');
    const pending = new Map<string, ReturnType<typeof setTimeout>>();
    // fire-and-forget async: the bridge resolves through the loop; the
    // publish lane never awaits it (VS Code's collection takes the result
    // whenever it lands, LAST-WINS on the debounce)
    const publish = (doc: vscode.TextDocument): void => {
        void analyzeDoc(rut, doc).then((a) => diagnostics.set(doc.uri, a.diags.map(toDiagnostic)));
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
            }, ANALYSIS_DEBOUNCE_MS),
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
            void rut.forget(uri);
            diagnostics.delete(doc.uri);
        }),
    );
    // documents already open at activation get their analysis now
    for (const doc of vscode.workspace.textDocuments) {
        schedule(doc);
    }

    // ---- the language features ----
    subscriptions.push(
        // the legend rides one async bridge call before registration
        await registerSemanticTokens(rut),
        registerDocumentSymbols(rut),
        registerHover(rut),
        registerCompletion(rut),
        registerDefinition(rut),
        registerTypeDefinition(rut),
        registerReferences(rut),
        registerInlayHints(rut),
        registerSignatureHelp(rut),
    );

    // the crate-shipped dep engine: debounce + one-run-in-flight +
    // rerun-once-if-stirred over `resolveDeps` — the extension's whole
    // dep lane. `roots` is re-read at every run (the folders the host
    // sees NOW), and `env` re-injects the read-only capability before
    // each run; the engine itself is a disposable the host retires.
    const engine = depEngine(rut, {
        env: hostEnv,
        roots: () => (vscode.workspace.workspaceFolders ?? []).map((f) => f.uri.fsPath),
    });
    subscriptions.push(engine);

    // the palette's manual re-index — the same lane activation and the
    // folder-change hook ride, on demand: covers watcher misses (a mount
    // a glob never saw, a walk that raced an edit)
    subscriptions.push(
        vscode.commands.registerCommand('rut.reindexDeps', () => {
            engine.stir();
        }),
    );

    // workspace index off the hot activation path — hover/completion may
    // briefly resolve against the prelude + open docs only. The dep
    // walk rides after it: the crate's read-only walk mounts the root
    // manifests' rows (path dirs, store-mounted url deps) into the same
    // index, through the injected env.
    watchDeps(context, rut, engine);
    void indexWorkspace(rut).then(() => engine.stir());
}

// `**/rut.jsonc` + `**/mod.rut` re-run the walk (a manifest landing or
// a mod tree moving re-mounts it); a workspace-folder change restarts
// the whole index (watchers re-arm over the new folder set, the scan +
// engine stir re-run — re-indexing is replace-by-origin, so this is
// idempotent). The extension is a pure READER: no write feedback loops
// exist anymore, so one debounce window is all the dynamics there are.
function watchDeps(context: vscode.ExtensionContext, rut: RutLsp, engine: { stir(): void }): void {
    const watchers: vscode.Disposable[] = [];
    const watchFolder = (folder: vscode.WorkspaceFolder): void => {
        for (const pattern of [`**/${MANIFEST_NAME}`, '**/mod.rut']) {
            const watcher = vscode.workspace.createFileSystemWatcher(
                new vscode.RelativePattern(folder, pattern),
            );
            watchers.push(
                watcher,
                watcher.onDidChange(() => engine.stir()),
                watcher.onDidCreate(() => engine.stir()),
                watcher.onDidDelete(() => engine.stir()),
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
            void indexWorkspace(rut).then(() => engine.stir());
        }),
        {
            dispose: () => {
                watchers.forEach((w) => {
                    w.dispose();
                });
            },
        },
    );
}
