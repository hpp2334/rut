/**
 * The AST tree renderer. The wasm `ast` payload is source of
 * truth — no display strings on the wire. Every label shown here is
 * derived: names/linkages verbatim, `vis`/`op` through small tables,
 * literal values sliced out of `source` by span.
 *
 * `rowsOf` is an exhaustive switch over the concrete node union — every
 * kind word `dump.rs` can emit has a case. A kind word this file hasn't
 * met yet (wire drift ahead of the d.ts) falls to the default, which
 * renders the raw fields as text rows — the pane degrades, never throws.
 */

import { memo, useEffect, useState } from "react";
import type {
  AstBound, AstFPart, AstMember, AstNode, AstSeg, AstStructField,
  BinOpTag, Span2, UnOpTag, VisTag,
} from "../wasm/rut-api";

// ---- derivation tables (the only display strings in this file) ----

const VIS_LABEL: Record<VisTag, string> = {
  pub: "pub",
  super: "pub(super)",
  pkg: "pub(pkg)",
  self: "private",
};

const OP_SYMBOL: Record<BinOpTag | UnOpTag, string> = {
  Add: "+", Sub: "-", Mul: "*", Div: "/", Mod: "%",
  Eq: "==", Ne: "!=", Lt: "<", Gt: ">", Le: "<=", Ge: ">=",
  And: "&&", Or: "||",
  BitAnd: "&", BitOr: "|", BitXor: "^", Shl: "<<", Shr: ">>",
  Neg: "-", Not: "!",
};

// ---- per-payload textOf helpers (no blob parameters) ----

const textOfSpan = (span: Span2, src: string): string =>
  src.slice(span[0], span[1]);

const textOfSegs = (segs: AstSeg[]): string =>
  segs.map((s) => s.name + (s.generics ? "<..>" : "")).join(".");

const textOfMember = (m: AstMember): string => m.ident;

const textOfBound = (w: AstBound): string => `${w.ident} requires`;

const textOfFPart = (p: AstFPart): string =>
  "str" in p ? JSON.stringify(p.str) : `{expr}`;

/** fallback text for a raw field value of an unmapped kind */
const rawText = (v: unknown): string => {
  if (typeof v === "string") return JSON.stringify(v);
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  if (Array.isArray(v)) return `[${v.length} items]`;
  if (v !== null && typeof v === "object") return "{..}";
  return String(v);
};

// ---- renderer rows ----

type Row =
  | { label: string; text: string } // scalar line:  label: <derived>
  | { label: string; node: AstNode } // nested node row
  | { label: string; text: string; node: AstNode } // labeled node line
  | { label: string; items: string[] } // bullet list of derived strings
  | { label: string; list: AstNode[] } // indented list of nodes
  | { label: string; fields: AstStructField[] };

function rowsOf(n: AstNode, src: string): Row[] {
  switch (n.kind) {
    case "Module":
      return [{ label: "items", list: n.items }];
    case "ModDecl":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "name", text: n.name },
      ];
    case "Use":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "path", items: n.path },
        { label: "names", items: n.names },
      ];
    case "Alias":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "name", text: n.name },
        { label: "target", node: n.target },
      ];
    case "ModuleLet":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "name", text: n.name },
        ...(n.ty ? [{ label: "ty", node: n.ty }] : []),
        { label: "init", node: n.init },
      ];
    case "Enum":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "name", text: n.name },
        { label: "members", items: n.members.map(textOfMember) },
      ];
    case "Class":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "name", text: n.name },
        ...(n.generics ? [{ label: "generics", items: n.generics }] : []),
        { label: "fields", list: n.fields },
        { label: "methods", list: n.methods },
        ...(n.newtype ? [{ label: "newtype", text: "true" }] : []),
        ...(n.bounds ? [{ label: "bounds", items: n.bounds.map(textOfBound) }] : []),
      ];
    case "Interface":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "name", text: n.name },
        ...(n.generics ? [{ label: "generics", items: n.generics }] : []),
        { label: "methods", list: n.methods },
      ];
    case "Impl":
      return [
        ...(n.generics ? [{ label: "generics", items: n.generics }] : []),
        { label: "target", node: n.target },
        { label: "methods", list: n.methods },
      ];
    case "Fn":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        ...(n.async ? [{ label: "async", text: "true" }] : []),
        { label: "name", text: n.name },
        ...(n.generics ? [{ label: "generics", items: n.generics }] : []),
        { label: "params", list: n.params },
        ...(n.ret ? [{ label: "ret", node: n.ret }] : []),
        ...(n.bounds ? [{ label: "bounds", items: n.bounds.map(textOfBound) }] : []),
        { label: "body", node: n.body },
      ];
    case "SurfaceFn":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "linkage", text: n.linkage },
        ...(n.is_async ? [{ label: "is_async", text: "true" }] : []),
        { label: "name", text: n.name },
        ...(n.generics ? [{ label: "generics", items: n.generics }] : []),
        { label: "params", list: n.params },
        ...(n.ret ? [{ label: "ret", node: n.ret }] : []),
      ];
    case "SurfaceStruct":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "name", text: n.name },
        { label: "fields", list: n.fields },
      ];
    case "BuiltinTy":
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "linkage", text: n.linkage },
        { label: "name", text: n.name },
        ...(n.generics ? [{ label: "generics", items: n.generics }] : []),
        { label: "members", list: n.members },
      ];
    case "BuiltinPrimitive":
      return [
        { label: "linkage", text: n.linkage },
        { label: "name", text: n.name },
        { label: "members", list: n.members },
      ];
    case "BuiltinImpl":
      return [
        { label: "prim", text: n.prim },
        { label: "methods", list: n.methods },
      ];
    case "Error":
      return n.diag ? [{ label: "diag", text: n.diag }] : [];
    case "FieldDecl":
      return [
        ...(n.vis ? [{ label: "vis", text: VIS_LABEL[n.vis] }] : []),
        ...(n.static ? [{ label: "static", text: "true" }] : []),
        { label: "name", text: n.name },
        { label: "ty", node: n.ty },
        ...(n.init ? [{ label: "init", node: n.init }] : []),
      ];
    case "MethodDecl":
      return [
        ...(n.vis ? [{ label: "vis", text: VIS_LABEL[n.vis] }] : []),
        ...(n.async ? [{ label: "async", text: "true" }] : []),
        ...(n.marker ? [{ label: "marker", text: n.marker }] : []),
        { label: "name", text: n.name },
        ...(n.generics ? [{ label: "generics", items: n.generics }] : []),
        { label: "params", list: n.params },
        ...(n.ret ? [{ label: "ret", node: n.ret }] : []),
        ...(n.bounds ? [{ label: "bounds", items: n.bounds.map(textOfBound) }] : []),
        ...(n.body ? [{ label: "body", node: n.body }] : []),
      ];
    case "Param":
      return [
        { label: "name", text: n.name },
        ...(n.ty ? [{ label: "ty", node: n.ty }] : []),
      ];
    case "SelfParam":
      return [];
    case "Block":
      return [
        { label: "stmts", list: n.stmts },
        ...(n.tail ? [{ label: "tail", node: n.tail }] : []),
      ];
    case "LetStmt":
      return [
        ...(n.const ? [{ label: "const", text: "true" }] : []),
        { label: "name", text: n.name },
        ...(n.destructure ? [{ label: "destructure", text: n.destructure }] : []),
        ...(n.ty ? [{ label: "ty", node: n.ty }] : []),
        { label: "init", node: n.init },
      ];
    case "If":
      return [
        { label: "cond", node: n.cond },
        { label: "then", node: n.then },
        ...(n.els ? [{ label: "els", node: n.els }] : []),
      ];
    case "While":
      return [
        { label: "cond", node: n.cond },
        { label: "body", node: n.body },
      ];
    case "ForOf":
      return [
        ...(n.const ? [{ label: "const", text: "true" }] : []),
        { label: "var", text: n.var },
        { label: "iter", node: n.iter },
        { label: "body", node: n.body },
      ];
    case "ForC":
      return [
        { label: "var", text: n.var },
        { label: "init", node: n.init },
        { label: "cond", node: n.cond },
        { label: "update", node: n.update },
        { label: "body", node: n.body },
      ];
    case "Return":
      return n.value ? [{ label: "value", node: n.value }] : [];
    case "Break":
    case "Continue":
    case "PatWild":
    case "PatElse":
      return [];
    case "WhenStmt":
      return [
        { label: "scrut", node: n.scrut },
        { label: "arms", list: n.arms },
      ];
    case "ExprStmt":
      return [{ label: "expr", node: n.expr }];
    case "WhenArm":
      return [
        { label: "pats", list: n.pats },
        { label: "body", node: n.body },
      ];
    case "PatLit":
      return [{ label: "expr", node: n.expr }];
    case "PatPath":
      return [{ label: "segs", items: [textOfSegs(n.segs)] }];
    case "TyPath":
      return [
        { label: "segs", items: [textOfSegs(n.segs)] },
      ];
    case "TyFn":
      return [
        { label: "params", list: n.params },
        { label: "ret", node: n.ret },
      ];
    case "TyOpt":
      return [{ label: "inner", node: n.inner }];
    case "TyArray":
      return [{ label: "elem", node: n.elem }];
    case "TyTuple":
      return [{ label: "elems", list: n.elems }];
    case "TyUnion":
      return [{ label: "elems", list: n.elems }];
    case "Lit":
      // the literal's own span slices back the exact source text
      return [{ label: "value", text: textOfSpan(n.span, src) }];
    case "Path":
      return [{ label: "segs", items: [textOfSegs(n.segs)] }];
    case "Call":
      return [
        { label: "callee", node: n.callee },
        { label: "args", list: n.args },
      ];
    case "Method":
      return [
        { label: "recv", node: n.recv },
        { label: "name", text: n.name },
        ...(n.generics ? [{ label: "generics", list: n.generics }] : []),
        { label: "args", list: n.args },
      ];
    case "Field":
      return [
        { label: "recv", node: n.recv },
        { label: "name", text: n.name },
      ];
    case "Index":
      return [
        { label: "recv", node: n.recv },
        { label: "idx", node: n.idx },
      ];
    case "Unary":
      return [
        { label: "op", text: OP_SYMBOL[n.op] },
        { label: "expr", node: n.expr },
      ];
    case "Binary":
      return [
        { label: "op", text: OP_SYMBOL[n.op] },
        { label: "lhs", node: n.lhs },
        { label: "rhs", node: n.rhs },
      ];
    case "Assign":
      return [
        ...(n.op ? [{ label: "op", text: OP_SYMBOL[n.op] }] : []),
        { label: "target", node: n.target },
        { label: "value", node: n.value },
      ];
    case "Lambda":
      return [
        { label: "params", list: n.params },
        ...(n.ret ? [{ label: "ret", node: n.ret }] : []),
        { label: "body", node: n.body },
      ];
    case "AsyncBlock":
      return [{ label: "body", node: n.body }];
    case "Await":
      return [{ label: "expr", node: n.expr }];
    case "FStr":
      return [{ label: "parts", items: n.parts.map(textOfFPart) }];
    case "Struct":
      // one wire kind, two shapes: the `{ .. }` literal expression vs
      // the field-only struct decl
      if ("ty" in n) {
        return [
          { label: "ty", node: n.ty },
          { label: "fields", fields: n.fields },
        ];
      }
      return [
        { label: "vis", text: VIS_LABEL[n.vis] },
        { label: "name", text: n.name },
        ...(n.generics ? [{ label: "generics", items: n.generics }] : []),
        { label: "fields", list: n.fields },
        { label: "methods", list: n.methods },
      ];
    case "ArrayLit":
      return [{ label: "elems", list: n.elems }];
    case "Tuple":
      return [{ label: "elems", list: n.elems }];
    case "ArrayRepeat":
      return [
        { label: "value", node: n.value },
        { label: "count", node: n.count },
      ];
    case "WhenExpr":
      return [
        { label: "scrut", node: n.scrut },
        { label: "arms", list: n.arms },
      ];
    case "Is":
    case "Cast":
      return [
        { label: "expr", node: n.expr },
        { label: "ty", node: n.ty },
      ];
    default: {
      // a wire kind word this file hasn't met (drift ahead of the d.ts):
      // render the raw fields as text rows — the pane degrades, never throws
      const raw = n as unknown as Record<string, unknown>;
      return Object.entries(raw)
        .filter(([k]) => k !== "id" && k !== "kind" && k !== "span")
        .map(([k, v]) => ({ label: k, text: rawText(v) }));
    }
  }
}

// ---- components ----

/** header summary for a collapsed row: the name field, else derived text */
function nodeSummary(n: AstNode, src: string): string {
  if ("name" in n && typeof n.name === "string") return n.name;
  if ("var" in n) return n.var;
  if (n.kind === "Lit") return textOfSpan(n.span, src);
  if (n.kind === "Path" && "segs" in n) return textOfSegs(n.segs);
  return "";
}

const NodeRow = memo(function NodeRow(props: {
  node: AstNode;
  src: string;
  depth: number;
  expandSignal: number;
  collapseSignal: number;
}): JSX.Element {
  const { node, src, depth, expandSignal, collapseSignal } = props;
  const [open, setOpen] = useState(depth < 1);
  useEffect(() => {
    if (expandSignal > 0) setOpen(true);
  }, [expandSignal]);
  useEffect(() => {
    if (collapseSignal > 0) setOpen(false);
  }, [collapseSignal]);
  // rows are cheap (plain arrays — no rendering until open); collapsed
  // subtrees never render their children
  const rows = rowsOf(node, src);
  const expandable = rows.length > 0;
  const summary = nodeSummary(node, src);
  return (
    <div className="ast-node" style={{ marginLeft: depth > 0 ? 14 : 0 }}>
      <div className="ast-head">
        {expandable ? (
          <button
            className="ast-toggle"
            aria-expanded={open}
            onClick={() => setOpen((v) => !v)}
          >
            {open ? "▾" : "▸"}
          </button>
        ) : (
          <span className="ast-leaf">·</span>
        )}
        <span className="ast-kind">
          @{node.id} {node.kind}
          {summary ? ` ${summary}` : ""}
        </span>
        <span className="ast-span">
          [{node.span[0]},{node.span[1]})
        </span>
      </div>
      {open && expandable && (
        <div className="ast-body">
          {rows.map((r, i) => (
            <div className="ast-field" key={i}>
              {"text" in r && <span className="ast-label">{r.label}:</span>}
              {"text" in r && <span className="ast-text">{r.text}</span>}
              {"items" in r && (
                <>
                  <span className="ast-label">{r.label}:</span>
                  <div className="ast-items">
                    {r.items.map((it, j) => (
                      <div className="ast-item" key={j}>- {it}</div>
                    ))}
                  </div>
                </>
              )}
              {"node" in r && (
                <>
                  <span className="ast-label">{r.label}:</span>
                  <NodeRow node={r.node} src={src} depth={depth + 1} expandSignal={expandSignal} collapseSignal={collapseSignal} />
                </>
              )}
              {"list" in r && (
                <>
                  <span className="ast-label">{r.label}:</span>
                  <div className="ast-list">
                    {r.list.map((c, j) => (
                      <NodeRow node={c} src={src} depth={depth + 1} expandSignal={expandSignal} collapseSignal={collapseSignal} key={j} />
                    ))}
                  </div>
                </>
              )}
              {"fields" in r && (
                <>
                  <span className="ast-label">{r.label}:</span>
                  <div className="ast-list">
                    {r.fields.map((f, j) => (
                      <div className="ast-item" key={j}>
                        - {f.ident}:
                        <NodeRow node={f.node} src={src} depth={depth + 1} expandSignal={expandSignal} collapseSignal={collapseSignal} />
                      </div>
                    ))}
                  </div>
                </>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
});

export function AstTree(props: { root: AstNode; source: string }): JSX.Element {
  const [expandSignal, setExpand] = useState(0);
  const [collapseSignal, setCollapse] = useState(0);
  return (
    <div className="ast-tree">
      <div className="ast-controls">
        <button onClick={() => setExpand((v) => v + 1)}>expand all</button>
        <button onClick={() => setCollapse((v) => v + 1)}>collapse all</button>
      </div>
      <NodeRow
        node={props.root}
        src={props.source}
        depth={0}
        expandSignal={expandSignal}
        collapseSignal={collapseSignal}
      />
    </div>
  );
}
