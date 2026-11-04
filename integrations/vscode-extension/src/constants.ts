// the extension's one set of budgets and names — mirrors the native
// server's caps so the two faces stay guests of the same size. The
// dep walk's own budgets (depth/files/fetch cap) are Rust-side now,
// in the crate that owns the walk.

import type * as vscode from 'vscode';

export const SELECTOR: vscode.DocumentSelector = { language: 'rut' };

// keystrokes coalesce: the last change inside the window wins (the
// stdio server got this from the language client's batching)
export const ANALYSIS_DEBOUNCE_MS = 200;

// the workspace-scan cap — an LSP is a guest,
// not an indexer daemon
export const MAX_WORKSPACE_FILES = 500;

// the manifest's one name — one directory is one module
export const MANIFEST_NAME = 'rut.jsonc';
