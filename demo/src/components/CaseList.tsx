import { memo } from "react";
import type { RutCase } from "../cases";

export interface CaseGroup {
  /** the group heading in the selector (e.g. "cases", "classics") */
  title: string;
  cases: RutCase[];
}

/** memo boundary (the M-1 fix): a source keystroke re-renders the
 * editor only — the case list bails (stable groups/id/callback). */
export const CaseList = memo(function CaseList(props: {
  groups: CaseGroup[];
  selectedId: string;
  onSelect: (c: RutCase) => void;
}): JSX.Element {
  return (
    <aside className="case-list">
      {props.groups.map((g) => (
        <section key={g.title}>
          <div className="case-list-title">{g.title}</div>
          <ul>
            {g.cases.map((c) => (
              <li key={c.id}>
                <button
                  className={c.id === props.selectedId ? "selected" : ""}
                  onClick={() => props.onSelect(c)}
                  title={c.blurb}
                >
                  <span className="case-name">{c.name}</span>
                  <span className="case-rfcs">{c.rfcs}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </aside>
  );
});
