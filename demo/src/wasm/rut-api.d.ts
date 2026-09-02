/**
 * The rut wasm contract (RFC 0041 §3).
 *
 * `demo/public/rut.wasm` (built from `crates/rut-wasm`, RFC 0041 §2) must
 * export these. This file is the single source of truth mirrored by
 * RFC 0041 §3 — change both together.
 */

export interface Diag {
  /** byte/char span in the source, if known */
  start?: number;
  end?: number;
  msg: string;
}

// ---- the AST tree (one interface per Rust `Kind` variant, 1:1) ----

export type Span2 = [number, number];
export type VisTag = "pub" | "mod" | "super" | "self";
export type LinkageTag = "host" | "extern";
export type BinOpTag =
  | "Add" | "Sub" | "Mul" | "Div" | "Mod"
  | "Eq" | "Ne" | "Lt" | "Gt" | "Le" | "Ge"
  | "And" | "Or"
  | "BitAnd" | "BitOr" | "BitXor" | "Shl" | "Shr"
  | "WrapAdd" | "WrapSub" | "WrapMul" | "WrapShl";
export type UnOpTag = "Neg" | "Not" | "BitNot";

// concrete companions (mirrors of the ast.rs / dump.rs companion types)
export interface AstSeg { name: string; generics?: AstNode[] }
export interface AstMember { ident: string; int?: number }
export interface AstStructField { ident: string; node: AstNode }
export interface AstWhere { ident: string; ty: AstNode }
export interface AstExtParam { ident: string; ty?: AstNode }
/** f-string part: a hole expression node, or a literal chunk */
export type AstFPart = AstNode | { kind: "FStrLit"; str: string };

interface Base { id: number; span: Span2 }

// ---- items ----
export interface AstModule extends Base { kind: "Module"; items: AstNode[] }
export interface AstImport extends Base { kind: "Import"; names: string[]; from: string }
export interface AstModuleLet extends Base { kind: "ModuleLet"; vis: VisTag; name: string; ty?: AstNode; init: AstNode }
export interface AstEnum extends Base { kind: "Enum"; vis: VisTag; name: string; members: AstMember[] }
export interface AstDataclass extends Base { kind: "Dataclass"; vis: VisTag; name: string; generics?: string[]; fields: AstNode[]; methods: AstNode[] }
export interface AstClass extends Base { kind: "Class"; vis: VisTag; name: string; generics?: string[]; fields: AstNode[]; methods: AstNode[] }
export interface AstTrait extends Base { kind: "Trait"; vis: VisTag; name: string; generics?: string[]; requires: AstNode[]; methods: AstNode[] }
export interface AstImpl extends Base { kind: "Impl"; trait: AstNode; target: AstNode; methods: AstNode[] }
export interface AstFn extends Base { kind: "Fn"; vis: VisTag; suspend?: true; name: string; generics?: string[]; params: AstNode[]; ret?: AstNode; wheres?: AstWhere[]; body: AstNode }
export interface AstSurfaceFn extends Base { kind: "SurfaceFn"; vis: VisTag; linkage: LinkageTag; name: string; generics?: string[]; params: AstNode[]; ret?: AstNode }
export interface AstSurfaceClass extends Base { kind: "SurfaceClass"; vis: VisTag; linkage: LinkageTag; name: string; extparams: AstExtParam[]; members: AstNode[] }

// ---- members & statements ----
export interface AstFieldDecl extends Base { kind: "FieldDecl"; private?: true; static?: true; name: string; ty: AstNode; init?: AstNode }
export interface AstMethodDecl extends Base { kind: "MethodDecl"; private?: true; suspend?: true; name: string; generics?: string[]; params: AstNode[]; ret?: AstNode; body?: AstNode }
export interface AstParam extends Base { kind: "Param"; mut?: true; name: string; ty?: AstNode }
export interface AstSelfParam extends Base { kind: "SelfParam"; mut?: true }
export interface AstBlock extends Base { kind: "Block"; stmts: AstNode[] }
export interface AstLetStmt extends Base { kind: "LetStmt"; mut?: true; name: string; ty?: AstNode; init: AstNode }
export interface AstIf extends Base { kind: "If"; cond: AstNode; then: AstNode; els?: AstNode }
export interface AstWhile extends Base { kind: "While"; cond: AstNode; body: AstNode }
export interface AstForOf extends Base { kind: "ForOf"; var: string; iter: AstNode; body: AstNode }
export interface AstForC extends Base { kind: "ForC"; var: string; init: AstNode; cond: AstNode; update: AstNode; body: AstNode }
export interface AstReturn extends Base { kind: "Return"; value?: AstNode }
export interface AstBreak extends Base { kind: "Break" }
export interface AstContinue extends Base { kind: "Continue" }
export interface AstWhenStmt extends Base { kind: "WhenStmt"; scrut: AstNode; arms: AstNode[] }
export interface AstExprStmt extends Base { kind: "ExprStmt"; expr: AstNode }

// ---- arms & patterns ----
export interface AstWhenArm extends Base { kind: "WhenArm"; pats: AstNode[]; body: AstNode }
export interface AstSelectArm extends Base { kind: "SelectArm"; fut: AstNode; bind?: string; body: AstNode }
export interface AstPatLit extends Base { kind: "PatLit"; expr: AstNode }
export interface AstPatPath extends Base { kind: "PatPath"; segs: AstSeg[] }
export interface AstPatCtor extends Base { kind: "PatCtor"; segs: AstSeg[]; args: (string | null)[] }
export interface AstPatWild extends Base { kind: "PatWild" }
export interface AstPatElse extends Base { kind: "PatElse" }

// ---- types ----
export interface AstTyPath extends Base { kind: "TyPath"; dyn?: true; segs: AstSeg[] }
export interface AstTyFn extends Base { kind: "TyFn"; params: AstNode[]; ret: AstNode }
export interface AstTyConst extends Base { kind: "TyConst"; expr: AstNode }

// ---- expressions ----
export interface AstLit extends Base { kind: "Lit" } // value derived: src.slice(span)
export interface AstPath extends Base { kind: "Path"; segs: AstSeg[] }
export interface AstCall extends Base { kind: "Call"; callee: AstNode; args: AstNode[] }
export interface AstMethodCall extends Base { kind: "Method"; recv: AstNode; name: string; generics?: AstNode[]; args: AstNode[] }
export interface AstFieldExpr extends Base { kind: "Field"; recv: AstNode; name: string }
export interface AstIndex extends Base { kind: "Index"; recv: AstNode; idx: AstNode }
export interface AstUnary extends Base { kind: "Unary"; op: UnOpTag; expr: AstNode }
export interface AstBinary extends Base { kind: "Binary"; op: BinOpTag; lhs: AstNode; rhs: AstNode }
export interface AstAssign extends Base { kind: "Assign"; op?: BinOpTag; target: AstNode; value: AstNode }
export interface AstLambda extends Base { kind: "Lambda"; params: AstNode[]; ret?: AstNode; body: AstNode }
export interface AstTry extends Base { kind: "Try"; expr: AstNode }
export interface AstFStr extends Base { kind: "FStr"; parts: AstFPart[] }
export interface AstStruct extends Base { kind: "Struct"; ty: AstNode; fields: AstStructField[] }
export interface AstArrayLit extends Base { kind: "ArrayLit"; elems: AstNode[] }
export interface AstWhenExpr extends Base { kind: "WhenExpr"; scrut: AstNode; arms: AstNode[] }
export interface AstAwait extends Base { kind: "Await"; expr: AstNode }
export interface AstSelect extends Base { kind: "Select"; arms: AstNode[] }
export interface AstIs extends Base { kind: "Is"; expr: AstNode; ty: AstNode }

export type AstNode =
  | AstModule | AstImport | AstModuleLet | AstEnum | AstDataclass | AstClass
  | AstTrait | AstImpl | AstFn | AstSurfaceFn | AstSurfaceClass
  | AstFieldDecl | AstMethodDecl | AstParam | AstSelfParam | AstBlock
  | AstLetStmt | AstIf | AstWhile | AstForOf | AstForC | AstReturn | AstBreak
  | AstContinue | AstWhenStmt | AstExprStmt
  | AstWhenArm | AstSelectArm | AstPatLit | AstPatPath | AstPatCtor
  | AstPatWild | AstPatElse
  | AstTyPath | AstTyFn | AstTyConst
  | AstLit | AstPath | AstCall | AstMethodCall | AstFieldExpr | AstIndex
  | AstUnary | AstBinary | AstAssign | AstLambda | AstTry | AstFStr
  | AstStruct | AstArrayLit | AstWhenExpr | AstAwait | AstSelect | AstIs;

export interface CompileResult {
  diags: Diag[];
  /** structured AST tree (source of truth — display strings are derived) */
  ast?: AstNode;
  /** pretty-printed LIR / bytecode (RFC 0032) */
  irDump: string;
  /** serialized module binary (RFC 0033) — absent when diags are fatal */
  binary?: Uint8Array;
}

export interface Budget {
  /** ops the run may execute (RFC 0040 §2) */
  fuel: number;
  /** bytes the self-managed heap may carve (RFC 0039/0040 §1) */
  heapBytes: number;
}

export interface RunResult {
  /** lines captured from the guest's print() calls */
  output: string[];
  /** "OutOfFuel" | "OutOfMemory" | "Interrupted" | trap kind, if any */
  trap?: string;
  fuelUsed: number;
  heapBytes: number;
}

export interface RutApi {
  compile(src: string): CompileResult;
  run(binary: Uint8Array, budget: Budget): RunResult;
}
