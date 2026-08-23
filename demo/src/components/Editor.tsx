import { useRef, useState } from "react";

/**
 * Minimal editor: textarea + line-number gutter. Tab inserts two spaces;
 * the gutter follows the textarea's scroll. No editor dependency
 * (RFC 0041 §3 — a CodeMirror upgrade is a noted path, not taken).
 */
export function Editor(props: {
  value: string;
  onChange: (src: string) => void;
}): JSX.Element {
  const [lineCount, setLineCount] = useState(countLines(props.value));
  const gutterRef = useRef<HTMLDivElement>(null);

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Tab") {
      e.preventDefault();
      const el = e.currentTarget;
      const { selectionStart: s, selectionEnd: en } = el;
      const next = el.value.slice(0, s) + "  " + el.value.slice(en);
      props.onChange(next);
      requestAnimationFrame(() => {
        el.selectionStart = el.selectionEnd = s + 2;
      });
    }
  };

  const onScroll = (e: React.UIEvent<HTMLTextAreaElement>) => {
    if (gutterRef.current) {
      gutterRef.current.scrollTop = e.currentTarget.scrollTop;
    }
  };

  const onChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    props.onChange(e.target.value);
    setLineCount(countLines(e.target.value));
  };

  const numbers: string[] = [];
  for (let i = 1; i <= lineCount; i++) numbers.push(String(i));

  return (
    <div className="editor">
      <div className="editor-gutter" ref={gutterRef}>
        <pre>{numbers.join("\n") + "\n"}</pre>
      </div>
      <textarea
        className="editor-input"
        spellCheck={false}
        value={props.value}
        onChange={onChange}
        onKeyDown={onKeyDown}
        onScroll={onScroll}
      />
    </div>
  );
}

function countLines(src: string): number {
  return src.split("\n").length;
}
