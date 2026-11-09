/**
 * The deps panel — the playground's face on the dep lane (phase 5):
 * the project's deps with their live status (the CDN boot fetching,
 * the walk's mounted/refused verdicts), the custom-dep add (a zip url
 * fetched, hashed, installed into the virtual store, declared in the
 * manifest + lock, and re-walked), and the walk's hint lane (the
 * env's `notify` messages — the same rows the extension surfaces).
 */

/** one dep row's live status */
export type DepStatus =
  | "fetching" // the CDN boot fetch is in flight
  | "installed" // the zip landed in the virtual store (pre-walk)
  | "mounted" // the walk's world carries it (no missing row)
  | "refused" // the walk refused it (absent, unpinned, or unverifiable)
  | "failed"; // the boot fetch/hash itself failed

export interface DepRowView {
  name: string;
  /** the manifest row's url */
  url: string;
  /** the full sha256 pin (the lock row; null while unpinned) */
  sha256: string | null;
  status: DepStatus;
  /** the refusal's reason (the WalkOut row / the fetch error) */
  note?: string;
  /** true when the row rode the std boot (not a custom add) */
  std: boolean;
}

const STATUS_LABEL: Record<DepStatus, string> = {
  fetching: "fetching…",
  installed: "installed",
  mounted: "mounted",
  refused: "refused",
  failed: "failed",
};

export function DepsPanel(props: {
  rows: DepRowView[];
  /** the boot lane's one-line state (the status line) */
  statusLine: string;
  /** the walk's notify hints, deduped, newest last */
  notices: string[];
  /** the custom-dep add: url → the lane (fetch → install → declare) */
  onAdd: (url: string) => void;
  /** the add lane's live state (null = idle) */
  addState: string | null;
}): JSX.Element {
  const mounted = props.rows.filter((r) => r.status === "mounted").length;
  return (
    <div className="deps-panel">
      <div className="deps-title">deps</div>
      <div className="deps-status">
        {props.statusLine}
        {props.rows.length > 0 && (
          <span className="deps-count">
            {" "}
            {mounted}/{props.rows.length} mounted
          </span>
        )}
      </div>
      <ul className="deps-list">
        {props.rows.map((r) => (
          <li key={r.url} className={`dep-row dep-${r.status}`}>
            <span className="dep-name">{r.name}</span>
            <span className="dep-pin">
              {r.sha256 ? `@${r.sha256.slice(0, 8)}` : "unpinned"}
            </span>
            <span className="dep-status">{STATUS_LABEL[r.status]}</span>
            {r.note && <span className="dep-note">{r.note}</span>}
          </li>
        ))}
      </ul>
      <form
        className="deps-add"
        onSubmit={(e) => {
          e.preventDefault();
          const input = (e.currentTarget.elements.namedItem("dep-url") as HTMLInputElement | null);
          const url = input?.value.trim() ?? "";
          if (url) {
            props.onAdd(url);
            if (input) input.value = "";
          }
        }}
      >
        <input
          name="dep-url"
          type="url"
          placeholder="https://cdn…/pkg.zip"
          aria-label="custom dep url"
          disabled={props.addState !== null}
        />
        <button type="submit" disabled={props.addState !== null}>
          add
        </button>
      </form>
      {props.addState && <div className="deps-add-state">{props.addState}</div>}
      {props.notices.length > 0 && (
        <ul className="deps-notices">
          {props.notices.slice(-3).map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
