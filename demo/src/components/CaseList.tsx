import type { RutCase } from "../cases";

export function CaseList(props: {
  cases: RutCase[];
  selectedId: string;
  onSelect: (c: RutCase) => void;
}): JSX.Element {
  return (
    <aside className="case-list">
      <div className="case-list-title">cases</div>
      <ul>
        {props.cases.map((c) => (
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
    </aside>
  );
}
