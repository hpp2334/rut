import { useState } from "react";
import type { AstNode, Diag } from "../wasm/rut-api";
import type { VerifyResult } from "../verify";
import { AstTree } from "./AstTree";

export interface PaneData {
  output: string[];
  ast?: AstNode;
  irDump: string;
  trap?: string;
  /** fatal compile diagnostics — rendered LOUD, never dropped silently */
  diags?: Diag[];
}

const TABS = ["Output", "AST", "IR"] as const;
type TabName = (typeof TABS)[number];

export function Panes(props: {
  data: PaneData;
  source: string;
  verify?: VerifyResult | null;
}): JSX.Element {
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
        <div className={cls("pane-body", active === "Output")}>
          {props.data.diags && props.data.diags.length > 0 && (
            <pre className="diags">
              {"compile diagnostics:"}
              {"\n"}
              {props.data.diags
                .map((d) => `  ✗ ${d.msg}${d.start != null ? ` (at ${d.start}..${d.end ?? d.start})` : ""}`)
                .join("\n")}
            </pre>
          )}
          {output.length ? (
            <pre>{output.join("\n")}</pre>
          ) : (
            !props.data.diags && <pre>(no output)</pre>
          )}
          {props.verify && !props.verify.ok && (
            <div className="diff">
              <div className="diff-head">
                ✗ differs from expected — line-paired (got vs sidecar):
              </div>
              <pre className="diff-body">
                {props.verify.rows
                  .map((r) => {
                    if (r.same) return `  = ${r.n} | ${r.got}`;
                    const got = r.got ?? "⟨missing⟩";
                    const exp = r.expected ?? "⟨no sidecar line⟩";
                    return `  ✗ ${r.n} | got:      ${got}\n    | expected: ${exp}`;
                  })
                  .join("\n")}
              </pre>
            </div>
          )}
        </div>
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
