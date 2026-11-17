/**
 * The rut wasm contract.
 *
 * `demo/public/rut.wasm` (built from `crates/rut-wasm` via wasm-pack)
 * carries the wasm-bindgen `RutEngine` class; its methods answer these
 * shapes. This file is the single source of truth — change both together.
 */

export interface Diag {
  /** byte/char span in the source, if known */
  start?: number;
  end?: number;
  msg: string;
}

// ---- the AST tree (one TS shape per `dump.rs` kind word, 1:1) ----

export type Span2 = [number, number];
export type VisTag = "pub" | "super" | "pkg" | "self";
/** the SurfaceFn `linkage` field — the wire emits exactly these two words */
export type LinkageTag = "host" | "builtin";
export type BinOpTag =
  | "Add" | "Sub" | "Mul" | "Div" | "Mod"
  | "Eq" | "Ne" | "Lt" | "Gt" | "Le" | "Ge"
  | "And" | "Or"
  | "BitAnd" | "BitOr" | "BitXor" | "Shl" | "Shr";
export type UnOpTag = "Neg" | "Not";

// concrete companions (mirrors of the dump.rs companion types)
export interface AstSeg { name: string; span: Span2; generics?: AstNode[] }
export interface AstMember { ident: string }
export interface AstStructField { ident: string; span: Span2; node: AstNode }
export interface AstBound { ident: string; ty: AstNode }
/** f-string part: a hole expression node, or a literal chunk */
export type AstFPart = AstNode | { kind: "FStrLit"; str: string };

interface Base { id: number; span: Span2 }

// ---- items ----
export interface AstModule extends Base { kind: "Module"; items: AstNode[] }
export interface AstModDecl extends Base { kind: "ModDecl"; vis: VisTag; name: string }
export interface AstUse extends Base { kind: "Use"; vis: VisTag; path: string[]; names: string[] }
export interface AstAlias extends Base { kind: "Alias"; vis: VisTag; name: string; target: AstNode }
export interface AstModuleLet extends Base { kind: "ModuleLet"; vis: VisTag; name: string; ty?: AstNode; init: AstNode }
export interface AstEnum extends Base { kind: "Enum"; vis: VisTag; name: string; members: AstMember[] }
export interface AstStructDecl extends Base { kind: "Struct"; vis: VisTag; name: string; generics?: string[]; fields: AstNode[]; methods: AstNode[] }
export interface AstClass extends Base { kind: "Class"; vis: VisTag; name: string; generics?: string[]; fields: AstNode[]; methods: AstNode[]; newtype?: true; bounds?: AstBound[] }
export interface AstInterface extends Base { kind: "Interface"; vis: VisTag; name: string; generics?: string[]; methods: AstNode[] }
export interface AstImpl extends Base { kind: "Impl"; generics?: string[]; target: AstNode; methods: AstNode[] }
export interface AstFn extends Base { kind: "Fn"; vis: VisTag; async?: true; name: string; generics?: string[]; params: AstNode[]; ret?: AstNode; bounds?: AstBound[]; body: AstNode }
export interface AstSurfaceFn extends Base { kind: "SurfaceFn"; vis: VisTag; linkage: LinkageTag; is_async?: true; name: string; generics?: string[]; params: AstNode[]; ret?: AstNode }
export interface AstSurfaceStruct extends Base { kind: "SurfaceStruct"; vis: VisTag; name: string; fields: AstNode[] }
export interface AstBuiltinTy extends Base { kind: "BuiltinTy"; vis: VisTag; linkage: string; name: string; generics?: string[]; members: AstNode[] }
export interface AstBuiltinPrimitive extends Base { kind: "BuiltinPrimitive"; linkage: string; name: string; members: AstNode[] }
export interface AstBuiltinImpl extends Base { kind: "BuiltinImpl"; prim: string; methods: AstNode[] }
export interface AstError extends Base { kind: "Error"; diag?: string } // one diag per message; JSON.parse keeps the last

// ---- members & statements ----
export interface AstFieldDecl extends Base { kind: "FieldDecl"; vis?: VisTag; static?: true; name: string; ty: AstNode; init?: AstNode }
export interface AstMethodDecl extends Base { kind: "MethodDecl"; vis?: VisTag; async?: true; marker?: string; name: string; generics?: string[]; params: AstNode[]; ret?: AstNode; bounds?: AstBound[]; body?: AstNode }
export interface AstParam extends Base { kind: "Param"; name: string; ty?: AstNode }
export interface AstSelfParam extends Base { kind: "SelfParam" }
export interface AstBlock extends Base { kind: "Block"; stmts: AstNode[]; tail?: AstNode }
export interface AstLetStmt extends Base { kind: "LetStmt"; const?: true; name: string; destructure?: string; ty?: AstNode; init: AstNode }
export interface AstIf extends Base { kind: "If"; cond: AstNode; then: AstNode; els?: AstNode }
export interface AstWhile extends Base { kind: "While"; cond: AstNode; body: AstNode }
export interface AstForOf extends Base { kind: "ForOf"; const?: true; var: string; iter: AstNode; body: AstNode }
export interface AstForC extends Base { kind: "ForC"; var: string; init: AstNode; cond: AstNode; update: AstNode; body: AstNode }
export interface AstReturn extends Base { kind: "Return"; value?: AstNode }
export interface AstBreak extends Base { kind: "Break" }
export interface AstContinue extends Base { kind: "Continue" }
export interface AstWhenStmt extends Base { kind: "WhenStmt"; scrut: AstNode; arms: AstNode[] }
export interface AstExprStmt extends Base { kind: "ExprStmt"; expr: AstNode }

// ---- arms & patterns ----
export interface AstWhenArm extends Base { kind: "WhenArm"; pats: AstNode[]; body: AstNode }
export interface AstPatLit extends Base { kind: "PatLit"; expr: AstNode }
export interface AstPatPath extends Base { kind: "PatPath"; segs: AstSeg[] }
export interface AstPatWild extends Base { kind: "PatWild" }
export interface AstPatElse extends Base { kind: "PatElse" }

// ---- types ----
export interface AstTyPath extends Base { kind: "TyPath"; segs: AstSeg[] }
export interface AstTyFn extends Base { kind: "TyFn"; params: AstNode[]; ret: AstNode }
export interface AstTyOpt extends Base { kind: "TyOpt"; inner: AstNode }
export interface AstTyArray extends Base { kind: "TyArray"; elem: AstNode }
export interface AstTyTuple extends Base { kind: "TyTuple"; elems: AstNode[] }
export interface AstTyUnion extends Base { kind: "TyUnion"; elems: AstNode[] }

// ---- expressions ----
export interface AstLit extends Base { kind: "Lit" } // value derived: src.slice(span)
export interface AstAsyncBlock extends Base { kind: "AsyncBlock"; body: AstNode }
export interface AstPath extends Base { kind: "Path"; segs: AstSeg[] }
export interface AstCall extends Base { kind: "Call"; callee: AstNode; args: AstNode[] }
export interface AstMethodCall extends Base { kind: "Method"; recv: AstNode; name: string; generics?: AstNode[]; args: AstNode[] }
export interface AstFieldExpr extends Base { kind: "Field"; recv: AstNode; name: string }
export interface AstIndex extends Base { kind: "Index"; recv: AstNode; idx: AstNode }
export interface AstUnary extends Base { kind: "Unary"; op: UnOpTag; expr: AstNode }
export interface AstBinary extends Base { kind: "Binary"; op: BinOpTag; lhs: AstNode; rhs: AstNode }
export interface AstAssign extends Base { kind: "Assign"; op?: BinOpTag; target: AstNode; value: AstNode }
export interface AstLambda extends Base { kind: "Lambda"; params: AstNode[]; ret?: AstNode; body: AstNode }
export interface AstFStr extends Base { kind: "FStr"; parts: AstFPart[] }
export interface AstStruct extends Base { kind: "Struct"; ty: AstNode; fields: AstStructField[] }
export interface AstTuple extends Base { kind: "Tuple"; elems: AstNode[] }
export interface AstArrayLit extends Base { kind: "ArrayLit"; elems: AstNode[] }
export interface AstArrayRepeat extends Base { kind: "ArrayRepeat"; value: AstNode; count: AstNode }
export interface AstWhenExpr extends Base { kind: "WhenExpr"; scrut: AstNode; arms: AstNode[] }
export interface AstAwait extends Base { kind: "Await"; expr: AstNode }
export interface AstIs extends Base { kind: "Is"; expr: AstNode; ty: AstNode }
export interface AstCast extends Base { kind: "Cast"; expr: AstNode; ty: AstNode }

export type AstNode =
  | AstModule | AstModDecl | AstUse | AstAlias | AstModuleLet | AstEnum
  | AstStructDecl | AstClass | AstInterface | AstImpl | AstFn | AstSurfaceFn
  | AstSurfaceStruct | AstBuiltinTy | AstBuiltinPrimitive | AstBuiltinImpl
  | AstError
  | AstFieldDecl | AstMethodDecl | AstParam | AstSelfParam | AstBlock
  | AstLetStmt | AstIf | AstWhile | AstForOf | AstForC | AstReturn | AstBreak
  | AstContinue | AstWhenStmt | AstExprStmt
  | AstWhenArm | AstPatLit | AstPatPath | AstPatWild | AstPatElse
  | AstTyPath | AstTyFn | AstTyOpt | AstTyArray | AstTyTuple | AstTyUnion
  | AstLit | AstAsyncBlock | AstPath | AstCall | AstMethodCall | AstFieldExpr
  | AstIndex | AstUnary | AstBinary | AstAssign | AstLambda | AstFStr
  | AstStruct | AstTuple | AstArrayLit | AstArrayRepeat | AstWhenExpr
  | AstAwait | AstIs | AstCast;

export interface CompileResult {
  diags: Diag[];
  /** structured AST tree (source of truth — display strings are derived) */
  ast?: AstNode;
  /** pretty-printed LIR / bytecode */
  irDump: string;
  /** serialized module binary — absent when diags are fatal */
  binary?: Uint8Array;
}

export interface Budget {
  /** ops the run may execute; 0 = uncapped (fuel disabled — the
   * mechanism default) */
  fuel: number;
  /** bytes the self-managed heap may carve */
  heapBytes: number;
}

export interface RunResult {
  /** lines captured from the guest's print() calls */
  output: string[];
  /** "OutOfFuel" | "OutOfMemory" | "Interrupted" | trap kind, if any */
  trap?: string;
  /**
   * a RETURNED failure (err-channel phase 3): a `main` that returned
   * the `(?T, err)` pair shape has its non-empty err surfaced here.
   * Distinct from `trap` by law — trap carries panics and budget
   * failures (bugs, drift); err carries data the page reads and acts on
   */
  err?: string;
  fuelUsed: number;
  heapBytes: number;
  /**
   * true when the guest trapped OutOfFuel with a live frame —
   * `resume` continues THAT frame; a fresh `run`
   * supersedes it (`dropFrame` retires it explicitly)
   */
  parked?: boolean;
}

export interface RutApi {
  compile(src: string): CompileResult;
  run(binary: Uint8Array, budget: Budget): RunResult;
  /**
   * Continue the parked frame with `extraFuel` more ops — the engine's
   * own park/resume (survey D5), NOT a re-run: locals and pc ride in
   * the machine, output ACCUMULATES across run+resumes, fuelUsed is
   * cumulative. Without a parked frame the trap names it (loud).
   */
  resume(extraFuel: number): RunResult;
  /** retire the parked frame (case switch); returns 1 if one was dropped */
  dropFrame(): number;
}
