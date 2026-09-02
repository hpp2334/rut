import { useState } from "react";
import type { AstNode } from "../wasm/rut-api";
import { AstTree } from "./AstTree";

export interface PaneData {
  output: string[];
  ast?: AstNode;
  irDump: string;
  trap?: string;
}

const TABS = ["Output", "AST", "IR"] as const;
type TabName = (typeof TABS)[number];

export function Panes(props: { data: PaneData; source: string }): JSX.Element {
  const [active, setActive] = useState<TabName>("Output");

  const output = [...props.data.output];
  if (props.data.trap) output.push(`Trap::${props.data.trap}`);

  return (
    <div className="panes">
      <div className="pane-bar" role="tablist">
        {TABS.map((name) => (
          <button
            key={name}
            role="tab"
            aria-selected={active === name}
            className={active === name ? "active" : ""}
            onClick={() => setActive(name)}
          >
            {name}
          </button>
        ))}
      </div>
      <div className="pane-bodies">
        <pre className={cls("pane-body", active === "Output")}>
          {output.length ? output.join("\n") : "(no output)"}
        </pre>
        <div className={cls("pane-body ast-pane", active === "AST")}>
          {props.data.ast ? (
            <AstTree root={props.data.ast} source={props.source} />
          ) : (
            <pre>(compile to see the AST tree)</pre>
          )}
        </div>
        <pre className={cls("pane-body", active === "IR")}>
          {props.data.irDump || "(compile to see the IR dump)"}
        </pre>
      </div>
    </div>
  );
}

function cls(base: string, visible: boolean): string {
  return visible ? `${base} visible` : base;
}
