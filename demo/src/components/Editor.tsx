import { Fragment, memo, useMemo, useRef, useState } from "react";
import type { OverlaySpan } from "../lsp/overlay";
import type { LspCompletionItem, LspDiag } from "../lsp/rut-lsp";

/** What the analyzer layer hands the editor: the overlay spans (token
 * classes + squiggle marks already cut) and the raw diags for the
 * hover-tip hit test. Null/absent = monochrome text (the honest
 * interim until the first analyze lands — survey D3). */
export interface Highlight {
  lines: OverlaySpan[][];
  diags: LspDiag[];
}

/**
 * the completion source — the editor popup's one query, answered by
 * the binding's `complete` (the deps-panel world included: std mounts
 * through the dep walk, so `use pouch::{ ⏐` offers `Vec` once the
 * CDN-fetched zips land in the virtual store)
 */
export type CompletionSource = (
  line: number,
  character: number,
  src: string,
) => Promise<LspCompletionItem[]>;

/** must match .editor-input's padding in styles.css */
const PAD_X = 12;
const PAD_Y = 10;
const PROBE_TEXT = "0000000000";

/** the characters whose keystroke opens the popup (LSP's trigger set:
 * a brace-list head, a `::` path, a `.` member) */
const TRIGGER_CHARS = new Set(["{", ":", "."]);
/** the max rows the popup lists before it scrolls */
const POPUP_MAX_ROWS = 9;

/** the popup's live state — the items, the selected row, and the
 * caret coordinates (editor-body-relative, already scroll-adjusted) */
interface CompletionPopup {
  items: LspCompletionItem[];
  sel: number;
  x: number;
  y: number;
  flipUp: boolean;
}

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
 * (the H-3 un-trap: Shift-Tab/with-modifier Tab keep the browser's
 * focus walk, and Escape blurs, so Run/Resume/budget/the pane tabs
 * stay keyboard-reachable). No editor dependency (a CodeMirror upgrade
 * is a noted path, not taken).
 *
 * The completion popup (the deps lane's face): the trigger chars and
 * Ctrl+Space open it, the walk-mounted world answers through the
 * `completions` source, and Enter/Tab/click inserts — the auto-import
 * tier's `additionalTextEdits` (the fresh `use` line) apply with it.
 */
export function Editor(props: {
  value: string;
  onChange: (src: string) => void;
  highlight?: Highlight | null;
  completions?: CompletionSource;
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
  const popupRef = useRef<HTMLDivElement>(null);

  // ---- the completion popup ----
  const [popup, setPopup] = useState<CompletionPopup | null>(null);
  /** the latest request's token — a stale answer never lands */
  const completionToken = useRef(0);

  /** the textarea's metrics (the diag tip's own math): char width off
   * the probe, line height off the computed style */
  const metrics = () => {
    const ta = inputRef.current;
    const probe = probeRef.current;
    if (!ta || !probe) return null;
    const lineH = parseFloat(getComputedStyle(ta).lineHeight);
    const probeW = probe.getBoundingClientRect().width;
    const charW = probeW / PROBE_TEXT.length;
    return lineH > 0 && charW > 0 ? { lineH, charW } : null;
  };

  /** (line, character) of the caret in `src` (0-based both) */
  const caretPos = (src: string, caret: number): [number, number] => {
    const before = src.slice(0, caret);
    const line = before.split("\n").length - 1;
    const ch = caret - (before.lastIndexOf("\n") + 1);
    return [line, ch];
  };

  /** fire one completion request at the caret; a thrown/empty answer
   * closes the popup. LAST-WINS on the token. */
  const requestCompletions = () => {
    const ta = inputRef.current;
    if (!ta || !props.completions) return;
    const src = ta.value;
    const m = metrics();
    if (!m) return;
    const caret = ta.selectionStart;
    const [line, ch] = caretPos(src, caret);
    // the popup anchors one row under the caret's visual position
    const body = bodyRef.current;
    const bodyH = body ? body.getBoundingClientRect().height : 0;
    const rows = popup?.items.length ?? POPUP_MAX_ROWS;
    const estH = Math.min(rows, POPUP_MAX_ROWS) * 22 + 8;
    const belowY = PAD_Y + (line + 1) * m.lineH - ta.scrollTop;
    const x = Math.min(
      Math.max(PAD_X + ch * m.charW - ta.scrollLeft, 4),
      (body?.clientWidth ?? 400) - 220,
    );
    const anchor = {
      x,
      y: belowY,
      flipUp: belowY + estH > bodyH && belowY - m.lineH - estH > 0,
    };
    const token = ++completionToken.current;
    void props
      .completions(line, ch, src)
      .then((items) => {
        if (token !== completionToken.current) return; // stale
        if (items.length === 0) {
          setPopup(null);
          return;
        }
        const y = anchor.flipUp
          ? PAD_Y + line * m.lineH - ta.scrollTop - estH
          : anchor.y;
        setPopup((prev) => ({
          items,
          sel: prev ? Math.min(prev.sel, items.length - 1) : 0,
          x: anchor.x,
          y,
          flipUp: anchor.flipUp,
        }));
      })
      .catch(() => {
        if (token === completionToken.current) setPopup(null);
      });
  };

  /** insert one item at the caret: the half-typed word before the
   * caret becomes the label, and the auto-import tier's
   * `additionalTextEdits` (a fresh `use pkg::{ Name };` line) apply
   * beside it — every edit against the ORIGINAL text, descending */
  const insertCompletion = (item: LspCompletionItem) => {
    const ta = inputRef.current;
    if (!ta) return;
    const src = ta.value;
    const s = ta.selectionStart;
    const wordStart = s - (src.slice(0, s).match(/[A-Za-z0-9_]*$/)?.[0]?.length ?? 0);
    type Edit = { start: number; end: number; text: string; isLabel: boolean };
    const edits: Edit[] = [
      { start: wordStart, end: s, text: item.label, isLabel: true },
      ...(item.additionalTextEdits ?? []).map((e) => ({
        start: indexAt(src, e.range.start.line, e.range.start.character),
        end: indexAt(src, e.range.end.line, e.range.end.character),
        text: e.newText,
        isLabel: false,
      })),
    ].sort((a, b) => b.start - a.start);
    let next = src;
    for (const e of edits) {
      next = next.slice(0, e.start) + e.text + next.slice(e.end);
    }
    // the caret lands after the inserted label, adjusted by every edit
    // that landed before it
    let caret = wordStart + item.label.length;
    for (const e of edits) {
      if (!e.isLabel && e.end <= wordStart) caret += e.text.length - (e.end - e.start);
    }
    setPopup(null);
    props.onChange(next);
    requestAnimationFrame(() => {
      ta.selectionStart = ta.selectionEnd = caret;
    });
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // the popup's keys while it is open — consumed before the plain
    // Tab/Escape lanes below
    if (popup) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = popup.items.length;
        setPopup({
          ...popup,
          sel: (popup.sel + (e.key === "ArrowDown" ? 1 : n - 1)) % n,
        });
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        insertCompletion(popup.items[popup.sel]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setPopup(null);
        return;
      }
    }
    // Ctrl+Space: the manual trigger — always requests
    if (e.key === " " && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      requestCompletions();
      return;
    }
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
    setPopup(null); // a scrolled popup misplaces the caret anchor
    hideTip();
  };

  const onChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const ta = e.target;
    props.onChange(ta.value);
    setLineCount(countLines(ta.value));
    // the auto-trigger: a trigger char opens the popup, a word
    // keystroke refreshes the open one, anything else closes it
    if (!props.completions) return;
    const caret = ta.selectionStart;
    const prevCh = caret > 0 ? ta.value[caret - 1] : "";
    const isWord = /[A-Za-z0-9_]/.test(prevCh);
    if (TRIGGER_CHARS.has(prevCh) || (popup !== null && isWord)) {
      requestCompletions();
    } else if (popup !== null) {
      setPopup(null);
    }
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
          onBlur={() => setPopup(null)}
        />
        <div className="editor-tip" ref={tipRef} hidden />
        {popup && (
          <div
            className="editor-complete"
            ref={popupRef}
            style={{ left: popup.x, top: popup.y }}
            role="listbox"
            aria-label="completions"
          >
            {popup.items.slice(0, POPUP_MAX_ROWS).map((item, i) => (
              <button
                key={`${item.label}:${i}`}
                type="button"
                role="option"
                aria-selected={i === popup.sel}
                className={i === popup.sel ? "selected" : undefined}
                // mousedown beats the textarea's blur — the click picks
                // without losing focus
                onMouseDown={(e) => {
                  e.preventDefault();
                  insertCompletion(item);
                }}
              >
                <span className="complete-label">{item.label}</span>
                {item.detail && (
                  <span className="complete-detail">{item.detail}</span>
                )}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** the absolute index of an LSP (line, character) in `src` — ASCII
 * docs make UTF-16 columns plain char counts; out-of-range positions
 * clamp (an edit against a doc the host never saw never panics) */
function indexAt(src: string, line: number, character: number): number {
  const lines = src.split("\n");
  let at = 0;
  for (let i = 0; i < line && i < lines.length; i++) at += lines[i].length + 1;
  return at + Math.max(0, Math.min(character, (lines[line]?.length ?? 0)));
}

function plainLines(src: string): OverlaySpan[][] {
  return src.split("\n").map((text) => [{ text }]);
}

function countLines(src: string): number {
  return src.split("\n").length;
}
