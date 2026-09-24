import { Fragment, memo, useMemo, useRef, useState } from "react";
import type { OverlaySpan } from "../lsp/overlay";
import type { LspDiag } from "../lsp/rut-lsp";

/** What the analyzer layer hands the editor: the overlay spans (token
 * classes + squiggle marks already cut) and the raw diags for the
 * hover-tip hit test. Null/absent = monochrome text (the honest
 * interim until the first analyze lands — survey D3). */
export interface Highlight {
  lines: OverlaySpan[][];
  diags: LspDiag[];
}

/** must match .editor-input's padding in styles.css */
const PAD_X = 12;
const PAD_Y = 10;
const PROBE_TEXT = "0000000000";

/** One overlay line (the M-1 memo boundary): the row bails unless its
 * span-array IDENTITY changed — the incremental builder in
 * lsp/overlay.ts keeps untouched lines' arrays identical across
 * analyzes, so a keystroke re-renders only the edited line instead of
 * re-reconciling the whole ~4 000-span subtree. Rows carry their
 * separating newline; the FINAL row's separator mirrors the textarea
 * value's own trailing newline (`eol`), so the pre's content matches
 * the value BYTE-FOR-BYTE in both cases (and the probe, riding after
 * the content, defeats the pre end-tag newline-swallow) — both layers'
 * scrollable heights then agree and the H-2 scroll-sync lands on exact
 * equality even at the very bottom of a long source. */
const OverlayLine = memo(function OverlayLine(props: {
  spans: OverlaySpan[];
  eol: boolean;
}): JSX.Element {
  return (
    <Fragment>
      {props.spans.map((sp, j) =>
        sp.type || sp.squiggle ? (
          <span
            key={j}
            className={sp.squiggle
              ? sp.type
                ? `tok-${sp.type} squiggle`
                : "squiggle"
              : `tok-${sp.type}`}
            title={sp.message}
          >
            {sp.text}
          </span>
        ) : (
          sp.text
        ),
      )}
      {props.eol ? "\n" : null}
    </Fragment>
  );
});

/**
 * The zero-dep double-layer editor (survey D4): a transparent-text
 * <textarea> (caret + selection live here) exactly over a highlighted
 * <pre> overlay (the analyzer's truth), same font metrics, scroll-
 * synced like the gutter. Tab inserts two spaces — UNMODIFIED Tab only
 * (the H-3 un-trap: Shift+Tab/with-modifier Tab keep the browser's
 * focus walk, and Escape blurs, so Run/Resume/budget/the pane tabs
 * stay keyboard-reachable). No editor dependency (RFC 0041 §3 — a
 * CodeMirror upgrade is a noted path, not taken).
 */
export function Editor(props: {
  value: string;
  onChange: (src: string) => void;
  highlight?: Highlight | null;
}): JSX.Element {
  const [lineCount, setLineCount] = useState(countLines(props.value));
  // the gutter follows the value on EVERY path: typing updates it in
  // onChange; an external swap (a case switch, a restore) re-syncs here
  // during render — without this the gutter kept the previous case's
  // line count until the next keystroke
  const propLineCount = countLines(props.value);
  if (propLineCount !== lineCount) setLineCount(propLineCount);

  const gutterRef = useRef<HTMLDivElement>(null);
  const overlayRef = useRef<HTMLPreElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const probeRef = useRef<HTMLSpanElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // the H-3 un-trap: capture ONLY unmodified Tab — Shift+Tab (and any
    // ctrl/meta chord) keeps the browser's focus walk, and Escape is
    // the forward escape hatch
    if (e.key === "Tab" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      const el = e.currentTarget;
      const { selectionStart: s, selectionEnd: en } = el;
      const next = el.value.slice(0, s) + "  " + el.value.slice(en);
      props.onChange(next);
      requestAnimationFrame(() => {
        el.selectionStart = el.selectionEnd = s + 2;
      });
    } else if (e.key === "Escape") {
      e.currentTarget.blur();
    }
  };

  const onScroll = (e: React.UIEvent<HTMLTextAreaElement>) => {
    const { scrollTop, scrollLeft } = e.currentTarget;
    if (gutterRef.current) gutterRef.current.scrollTop = scrollTop;
    if (overlayRef.current) {
      overlayRef.current.scrollTop = scrollTop;
      overlayRef.current.scrollLeft = scrollLeft;
    }
    hideTip();
  };

  const onChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    props.onChange(e.target.value);
    setLineCount(countLines(e.target.value));
  };

  // ---- the diagnostics hover tip (zero-dep): monospace metrics turn
  // the mouse position into an LSP position; a covering diag shows the
  // tip. The overlay itself is pointer-transparent so the caret never
  // loses a click.
  const hideTip = () => {
    if (tipRef.current) tipRef.current.hidden = true;
  };

  const onMouseMove = (e: React.MouseEvent<HTMLTextAreaElement>) => {
    const diags = props.highlight?.diags;
    const ta = inputRef.current;
    const body = bodyRef.current;
    const tip = tipRef.current;
    if (!diags || diags.length === 0 || !ta || !body || !tip) return;
    const probe = probeRef.current;
    const lineH = probe
      ? parseFloat(getComputedStyle(ta).lineHeight)
      : NaN;
    const probeW = probe ? probe.getBoundingClientRect().width : NaN;
    const charW = probeW / PROBE_TEXT.length;
    if (!(lineH > 0) || !(charW > 0)) return;

    const rect = ta.getBoundingClientRect();
    const col = Math.round(
      (e.clientX - rect.left + ta.scrollLeft - PAD_X) / charW,
    );
    const line = Math.floor((e.clientY - rect.top + ta.scrollTop - PAD_Y) / lineH);
    const hit = diags.find((d) => {
      if (d.range.start.line !== line) return false;
      const c0 = d.range.start.character;
      const c1 =
        d.range.end.line === line
          ? d.range.end.character
          : Number.MAX_SAFE_INTEGER;
      return col >= c0 && col < c1;
    });
    if (!hit) {
      tip.hidden = true;
      return;
    }
    const bodyRect = body.getBoundingClientRect();
    tip.textContent = `${hit.message} — line ${line + 1}`;
    tip.style.left = `${e.clientX - bodyRect.left + 14}px`;
    tip.style.top = `${e.clientY - bodyRect.top + 18}px`;
    tip.hidden = false;
  };

  // the gutter text + the row array: memoized so a keystroke that
  // changes neither skips recomputing them
  const numbersText = useMemo(() => {
    const numbers: string[] = [];
    for (let i = 1; i <= lineCount; i++) numbers.push(String(i));
    return numbers.join("\n") + "\n";
  }, [lineCount]);

  const lines = useMemo(
    () => (props.highlight ? props.highlight.lines : plainLines(props.value)),
    [props.highlight, props.value],
  );
  // the final row's separator mirrors the value's trailing newline so
  // the overlay's content is the value byte-for-byte
  const valueEndsNl = props.value.charCodeAt(props.value.length - 1) === 10;

  return (
    <div className="editor">
      <div className="editor-gutter" ref={gutterRef}>
        <pre>{numbersText}</pre>
      </div>
      <div
        className="editor-body"
        ref={bodyRef}
        onMouseLeave={hideTip}
      >
        <pre className="editor-overlay" ref={overlayRef} aria-hidden>
          {lines.map((spans, i) => (
            <OverlayLine
              key={i}
              spans={spans}
              eol={i < lines.length - 1 || valueEndsNl}
            />
          ))}
          <span ref={probeRef} className="editor-probe">
            {PROBE_TEXT}
          </span>
        </pre>
        <textarea
          className="editor-input"
          ref={inputRef}
          spellCheck={false}
          wrap="off"
          value={props.value}
          onChange={onChange}
          onKeyDown={onKeyDown}
          onScroll={onScroll}
          onMouseMove={onMouseMove}
        />
        <div className="editor-tip" ref={tipRef} hidden />
      </div>
    </div>
  );
}

function plainLines(src: string): OverlaySpan[][] {
  return src.split("\n").map((text) => [{ text }]);
}

function countLines(src: string): number {
  return src.split("\n").length;
}
