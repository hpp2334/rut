/**
 * The overlay builder (survey D3/D4): turns one analyze response into
 * per-line spans for the editor's highlight layer. Pure — no React, no
 * DOM — so the smoke can drive the exact builder the browser paints.
 *
 * - token classes come from the `rut_legend()` NAMES (index -> name ->
 *   `tok-<name>` in styles.css); the order is never hardcoded here
 * - diagnostics become squiggle marks carrying their message (the
 *   editor's hover tip / the span title)
 * - plain text carries no mark and paints the default color
 */

import type { DecodedToken, LspDiag } from "./rut-lsp";

export interface OverlaySpan {
  text: string;
  /** legend name — renders as class `tok-<name>` when present */
  type?: string;
  /** a diagnostic covers this range — wavy underline */
  squiggle?: boolean;
  /** the covering diagnostic's message (tooltip text) */
  message?: string;
}

export interface OverlayMark {
  start: number;
  end: number;
  type?: string;
  message?: string;
}

/** The per-analyze overlay cache (the M-1 fix): the previous source's
 * lines + marks. The incremental builder re-cuts ONLY the lines whose
 * text or marks changed — every untouched line keeps its span-array
 * identity, and the editor's memoized overlay row skips re-rendering. */
export interface OverlayCache {
  srcLines: string[];
  lines: OverlaySpan[][];
  tokMarks: OverlayMark[][];
  diagMarks: OverlayMark[][];
}

interface Mark {
  start: number;
  end: number;
  type?: string;
  message?: string;
}

/** Build the overlay: one span array per source line. */
export function buildOverlay(
  source: string,
  tokens: DecodedToken[],
  legend: string[],
  diags: LspDiag[],
): OverlaySpan[][] {
  return buildOverlayCached(
    source,
    tokens,
    legend,
    diags,
    undefined,
  ).lines;
}

/** The incremental shape of `buildOverlay` (the M-1 fix): identical
 * output, but line arrays survive untouched across analyzes when a
 * previous cache is handed in. */
export function buildOverlayCached(
  source: string,
  tokens: DecodedToken[],
  legend: string[],
  diags: LspDiag[],
  prev: OverlayCache | undefined,
): OverlayCache {
  const lines = source.split("\n");
  const tokMarks: Mark[][] = lines.map(() => []);
  const diagMarks: Mark[][] = lines.map(() => []);

  // tokens are single-line by construction (LSP semantic tokens)
  for (const t of tokens) {
    if (t.line < 0 || t.line >= lines.length) continue;
    const len = lines[t.line].length;
    const start = clamp(t.start, 0, len);
    const end = clamp(t.start + t.length, start, len);
    if (end > start) {
      tokMarks[t.line].push({ start, end, type: legend[t.type] });
    }
  }

  // diagnostics may span lines; each covered line gets its clamp
  for (const d of diags) {
    const from = clamp(d.range.start.line, 0, lines.length - 1);
    const to = clamp(d.range.end.line, from, lines.length - 1);
    for (let ln = from; ln <= to; ln++) {
      const len = lines[ln].length;
      const start = clamp(ln === from ? d.range.start.character : 0, 0, len);
      const end = clamp(ln === to ? d.range.end.character : len, start, len);
      if (end > start) {
        diagMarks[ln].push({ start, end, message: d.message });
      }
    }
  }

  const out = lines.map((text, ln) => {
    // the line's text AND both mark sets unchanged -> keep the previous
    // array identity (the memoized overlay row then bails); everything
    // else is re-cut exactly as the fresh path would
    const prevLines = prev?.lines;
    const prevSrcLines = prev?.srcLines;
    const prevTok = prev?.tokMarks;
    const prevDiag = prev?.diagMarks;
    if (
      prevLines !== undefined &&
      prevSrcLines !== undefined &&
      prevTok !== undefined &&
      prevDiag !== undefined &&
      prevLines[ln] !== undefined &&
      prevSrcLines[ln] === text &&
      marksEqual(prevTok[ln] ?? [], tokMarks[ln]) &&
      marksEqual(prevDiag[ln] ?? [], diagMarks[ln])
    ) {
      return prevLines[ln];
    }
    return cutLine(text, tokMarks[ln], diagMarks[ln]);
  });
  return { srcLines: lines, lines: out, tokMarks, diagMarks };
}

function marksEqual(a: OverlayMark[], b: OverlayMark[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i];
    const y = b[i];
    if (
      x.start !== y.start ||
      x.end !== y.end ||
      x.type !== y.type ||
      x.message !== y.message
    ) {
      return false;
    }
  }
  return true;
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

/** Cut one line at every mark boundary; each segment inherits the
 * token class and squiggle mark of the ranges that cover it. */
function cutLine(text: string, toks: Mark[], diags: Mark[]): OverlaySpan[] {
  const cuts = new Set<number>([0, text.length]);
  for (const m of toks) {
    cuts.add(m.start);
    cuts.add(m.end);
  }
  for (const m of diags) {
    cuts.add(m.start);
    cuts.add(m.end);
  }
  const bounds = [...cuts].sort((a, b) => a - b);
  const spans: OverlaySpan[] = [];
  for (let i = 0; i < bounds.length - 1; i++) {
    const start = bounds[i];
    const end = bounds[i + 1];
    if (end <= start) continue;
    const tok = toks.find((m) => m.start <= start && end <= m.end);
    const diag = diags.find((m) => m.start <= start && end <= m.end);
    spans.push({
      text: text.slice(start, end),
      type: tok?.type,
      squiggle: diag !== undefined || undefined,
      message: diag?.message,
    });
  }
  return spans.length > 0 ? spans : [{ text }];
}
