// LSP (JSON) -> vscode mapping.
// LSP protocol numbers are 1-based; the vscode enums are 0-based, so the
// mapping is `- 1` across the board (severity, symbol kind, completion
// kind).

import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import {
  LspCompletionItem,
  LspDiag,
  LspInlayHint,
  LspLocation,
  LspRange,
  LspSignatureHelp,
  LspSymbol,
} from './wasm';
import { hintOnce, workspaceHasZipCache } from './util';

// a definition target -> a vscode Location. A target with a scheme
// parses as-is; a repo-relative path (the embedded std surface's true
// source, e.g. `rut/pouch/src/mod.rut`) resolves against the workspace
// folders — a workspace that is the rut repo gets a real jump into the
// stdlib. Nowhere to resolve: dropped (never a wrong jump). A FULLY
// dropped answer — the wasm resolved the symbol but every target is
// unopenable (a url dep not extracted yet, mostly) — hints once per
// document instead of a silent dead ctrl+click; an empty input stays
// silent (an honest miss — nothing resolved at all).
export function resolveLocations(locs: LspLocation[], srcUri: string): vscode.Location[] {
  const out: vscode.Location[] = [];
  for (const l of locs) {
    const uri = toTargetUri(l.uri);
    if (uri) {
      out.push(new vscode.Location(uri, toRange(l.range)));
    }
  }
  if (locs.length > 0 && out.length === 0) {
    hintOnce(`def-dropped:${srcUri}`, defDropHint(locs), false);
  }
  return out;
}

// the drop hint names the LIKELIEST cause, not just the generic one.
// A relative `rut/…` target is the embedded std surface answering for a
// dep that never mounted; when the workspace's cache carries source
// zips, `rut install` already ran and reloading cannot help — the
// definition lives in the rut repo sources, so the hint says so.
function defDropHint(locs: LspLocation[]): string {
  if (locs.every((l) => l.uri.startsWith('rut/')) && workspaceHasZipCache()) {
    return 'rut: definition target not openable — the resolved definition lives in the rut ' +
      'repo sources, which are not in this workspace; reload the window if the dep was just installed';
  }
  return 'rut: definition target not openable — a dep may not be extracted yet; ' +
    'run `rut install`, then reload the window';
}

export function toTargetUri(raw: string): vscode.Uri | undefined {
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

export function toRange(r: LspRange): vscode.Range {
  return new vscode.Range(r.start.line, r.start.character, r.end.line, r.end.character);
}

export function toDiagnostic(d: LspDiag): vscode.Diagnostic {
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

export function toSymbol(s: LspSymbol): vscode.DocumentSymbol {
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

export function toCompletionItem(c: LspCompletionItem): vscode.CompletionItem {
  const item = new vscode.CompletionItem(c.label, ((c.kind ?? 1) - 1) as vscode.CompletionItemKind);
  if (c.detail) {
    item.detail = c.detail;
  }
  if (c.documentation) {
    item.documentation = new vscode.MarkdownString(c.documentation.value);
  }
  // the auto-import tier: sorts after the locals, and accepting the
  // item inserts `use mod::Name;` as a fresh line (the engine computed
  // both over the normalized source — the extension only maps them)
  if (c.sortText) {
    item.sortText = c.sortText;
  }
  if (c.additionalTextEdits) {
    item.additionalTextEdits = c.additionalTextEdits.map((e) => new vscode.TextEdit(toRange(e.range), e.newText));
  }
  return item;
}

// LSP and vscode share the inlay-kind numbering (1 Type, 2 Parameter)
// — passed through as-is, unlike the 1-based-vs-0-based enums above.
export function toInlayHint(h: LspInlayHint): vscode.InlayHint {
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

export function toSignatureHelp(h: LspSignatureHelp): vscode.SignatureHelp {
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
