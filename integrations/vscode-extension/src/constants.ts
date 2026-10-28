// the extension's one set of budgets and names — mirrors the native
// server's caps so the two faces stay guests of the same size

import * as vscode from 'vscode';

export const SELECTOR: vscode.DocumentSelector = { language: 'rut' };

// keystrokes coalesce: the last change inside the window wins (the
// stdio server got this from the language client's batching)
export const ANALYSIS_DEBOUNCE_MS = 200;

// mirrors the native server's workspace-scan cap — an LSP is a guest,
// not an indexer daemon
export const MAX_WORKSPACE_FILES = 500;

// the manifest's one name — one directory is one module
export const MANIFEST_NAME = 'rut.jsonc';

// the dep walk's budget beside the 500-file workspace cap: depth for
// the recursion, files for everything the walk indexes
export const MAX_DEP_DEPTH = 8;
export const MAX_DEP_FILES = 200;

// the fetch cap — the same bytes ceiling the CLI's remote enforces
export const MAX_FETCH_BYTES = 256 * 1024 * 1024;
