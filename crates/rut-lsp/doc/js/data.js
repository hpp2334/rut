/* The canned content — every example on the page, lifted from the
 * crate's real unit-test fixtures (hover.rs, completion.rs). What the
 * reader clicks is what the tests pin. */
import { html } from "./vdom.js";

/* completion lab ------------------------------------------------------ */
export const LAB_CODE = `// inventory.rut — a tour of the completion tiers
use json::{ encodeJson };

enum Color {
  Red;
  Green;
}

struct Counter {
  n: i32;
}

impl Counter {
  fn bump(self) -> i32 {
    return self.n + 1;
  }
  fn reset(self) -> nil { }
}

fn label(c: Color) -> str {
  return "color";
}

entry fn main() -> nil {
  let count:  = 42;
  let c = Counter::new(0);
  let text = f"n is {count}";
  
  c.
  let col = Color::;
}`;

export const LAB = [
  {
    id: "use", ln: 2, after: "use json::", title: "the use tier",
    edge: "Use", why: html`The cursor sits inside a <code>use</code> statement, right after
      <code>json::</code>. The tier walks that package and lists its surface — only
      <code>pub</code> names cross a package boundary (fns, types, lets and
      <code>pub&nbsp;use</code> re-exports all ride the same gate).`,
    items: [
      { label: "encodeJson", kind: "Fn", detail: "pub fn" },
      { label: "decodeJson", kind: "Fn", detail: "pub fn" },
      { label: "JsonI64", kind: "Type", detail: "pub type" },
      { label: "JsonStr", kind: "Type", detail: "pub type" },
      { label: "JsonArr", kind: "Type", detail: "pub type" },
      { label: "SOpt", kind: "Type", detail: "pub type" },
    ],
    note: html`At <code>use ⏐</code> — nothing walked yet — the tier lists the mounted
      <b>module names</b> instead (every named index; nothing is hardcoded).`,
  },
  {
    id: "ann", ln: 25, after: "let count: ", title: "the type tier (annotation boundary)",
    edge: "Annotation", why: html`<code>let count: ⏐</code> is a type slot — the <code>:</code>
      boundary (one text probe; the tree records no punct there) names the tier. Primitives,
      the document's own types, ambient builtin rows and imported types complete.`,
    items: [
      { label: "i32", kind: "Type", detail: "primitive" },
      { label: "i64", kind: "Type", detail: "primitive" },
      { label: "f64", kind: "Type", detail: "primitive" },
      { label: "str", kind: "Type", detail: "primitive" },
      { label: "bool", kind: "Type", detail: "primitive" },
      { label: "Color", kind: "Type", detail: "declared here" },
      { label: "Counter", kind: "Type", detail: "declared here" },
    ],
    note: html`No keywords, no fns, no locals — <b>values never ride a type position</b>.
      (Params and return arrows ride the same tier: <code>fn f(x: ⏐</code>, <code>-> ⏐</code>.)`,
  },
  {
    id: "bare", ln: 28, after: "  ", title: "the bare tiers (merged)",
    edge: "Bare", why: html`A plain expression position with nothing typed. Lists merge in
      precedence order and dedupe by label — first occurrence wins:`,
    groups: [
      ["locals — the bindings pass, shadow-correct", [
        { label: "count", kind: "Variable", detail: "let — i32" },
        { label: "c", kind: "Variable", detail: "let — Counter" },
        { label: "text", kind: "Variable", detail: "let — str" },
      ]],
      ["keywords — the grammar's reserved set", [
        { label: "let", kind: "Keyword" }, { label: "fn", kind: "Keyword" },
        { label: "use", kind: "Keyword" }, { label: "struct", kind: "Keyword" },
        { label: "impl", kind: "Keyword" }, { label: "enum", kind: "Keyword" },
        { label: "entry", kind: "Keyword" }, { label: "…", kind: "Keyword" },
      ]],
      ["visible decls — the bare-name law", [
        { label: "Color", kind: "Type", detail: "own decl" },
        { label: "Counter", kind: "Type", detail: "own decl" },
        { label: "label", kind: "Fn", detail: "own decl" },
        { label: "main", kind: "Fn", detail: "own decl" },
      ]],
      ["imports — bound by this document's use", [
        { label: "encodeJson", kind: "Fn", detail: "from json" },
      ]],
      ["auto-import — direct deps only, sorts last", [
        { label: "decodeJson", kind: "Fn", insert: "use json::{ decodeJson };", detail: "not imported yet" },
      ]],
    ],
    note: html`The <b>dot guard</b>: a bare request sitting right after a <code>.</code> the
      tree couldn't shape is a member request with an unresolved receiver — the answer is
      <b>empty</b>, never the keyword dump.`,
  },
  {
    id: "member", ln: 29, after: "c.", title: "the member tier",
    edge: "Member", why: html`<code>c.</code> mid-typing is an <b>incomplete node</b> in the
      tree — a method node whose member slot is absent — so the cursor is ON the member
      position. The receiver <code>c</code> resolves through its initializer →
      <code>Counter</code>, and the member law answers:`,
    items: [
      { label: "n", kind: "Field", detail: "n: i32" },
      { label: "bump", kind: "Method", detail: "fn bump(self) -> i32" },
      { label: "reset", kind: "Method", detail: "fn reset(self) -> nil" },
    ],
    note: html`An unresolved receiver is an <b>empty</b> tier, never a wrong one. A receiver
      that names a module (<code>helpers::mk(</code>) diverts to the position-path tier with
      visibility gates.`,
  },
  {
    id: "qual", ln: 30, after: "Color::", title: "the :: tier — an enum's members",
    edge: "Qualified", why: html`<code>Color::⏐</code> — the head resolves to a type in the
      index, and an enum's member list is the answer. A resolved head is <b>final</b>: the
      list is the answer even when empty.`,
    items: [
      { label: "Red", kind: "EnumMember", detail: "Red;" },
      { label: "Green", kind: "EnumMember", detail: "Green;" },
    ],
    note: html`An interface head offers nothing (its members ride receivers). A non-enum type
      head lists statics + associated fns — try the <code>Counter::</code> dot on line 26.`,
  },
  {
    id: "qualfn", ln: 26, after: "Counter::", title: "the :: tier — statics & associated fns",
    edge: "Qualified", why: html`<code>Counter::⏐</code> — a non-enum type head lists its
      static fields plus the inherent <code>impl Counter</code> fns. The constructor
      <code>new</code> lives exactly there.`,
    items: [
      { label: "new", kind: "Fn", detail: "fn new(n: i32) -> Counter" },
    ],
    note: html`And a head that resolves to a <b>module</b> of your own package? That is the
      position-path tier: the module's members filtered by what this file may see —
      private reaches the module's subtree, pub(pkg) the package, pub everywhere.`,
  },
];

/* hover demo ------------------------------------------------------------ */
export const HOVER_CODE = `// a 2D point
struct Point {
  x: f64;
  y: f64;
}

// euclidean length
fn length(p: Point) -> f64 {
  return p.x;
}`;

export const HOVER_TARGETS = [
  { ln: 8, tok: "p", label: "p — a parameter",
    pre: "p: Point", where: "parameter · fn length" },
  { ln: 8, tok: "Point", label: "Point — a type use",
    pre: "struct Point {\n  x: f64;\n  y: f64;\n}", where: "declared here · line 2", doc: "a 2D point" },
  { ln: 9, tok: "x", label: "x — a field, reached through its receiver",
    pre: "x: f64", where: "in `Point`" },
];

/* query cards ------------------------------------------------------------ */
export const QUERIES = [
  { icon: "pulse", name: "analyze", sig: "analyze(uri, src) → { diags, tokens, symbols }",
    ex: 'await lsp.analyze(uri, src)\n// → {"diags":[…],"tokens":{"data":[…]},"symbols":[…]}',
    d: "The editor heartbeat: diagnostics (parse errors + type checks), the semantic-token stream for coloring, and the symbol tree for the outline." },
  { icon: "search", name: "hover", sig: "hover(uri, line, ch) → Hover | null",
    ex: "await lsp.hover(uri, 9, 15)   // → markdown, verbatim",
    d: "What is under the cursor — rendered as the declaration's own text, never a pretty-printer. Try it below." },
  { icon: "arrow", name: "definition · typeDefinition", sig: "definition(uri, line, ch) → Location[]",
    ex: 'await lsp.definition(uri, 9, 15)\n// → [{ "uri": "file:///…/json/src/mod.rut", … }]',
    d: "Jumps resolve through locals, the collection doors, the member law, and the use-graph — across files, into mounted deps, no re-parsing." },
  { icon: "link", name: "references", sig: "references(uri, line, ch, incl) → Location[]",
    ex: "one scan across EVERY mounted file — each name\nresolves in its own file's context (chain reordered)",
    d: "Every use of the symbol, across the whole world. Shadow-correct: an inner re-declaration never matches the outer name." },
  { icon: "tag", name: "inlay", sig: "inlay(uri, range) → InlayHint[]",
    ex: "let c = Counter::new(0);   // : Counter ← inferred\nlog.info(msg);           // msg: ← param name",
    d: "A whole-document pass computed once per version (lazy), the viewport is just a filter. Inferred text is the one sanctioned source-less render." },
  { icon: "parens", name: "signatureHelp", sig: "signature_help(uri, line, ch) → SignatureHelp",
    ex: "Counter::new(⏐42)   → fn new(n: i32) -> Counter\n                     ^ active param: n",
    d: "Works mid-typing, inside unclosed calls — the incomplete call is a real node in the tree, not an error." },
  { icon: "spark", name: "complete", sig: "complete(uri, src, line, ch) → CompletionItem[]",
    ex: "six tiers, one probe — the next section is a\nplayground you can click through",
    d: "Takes the source on every keystroke (the doc-sync law): completion must answer for text analyze hasn't seen yet." },
];

/* overview / world / member / wire / map ---------------------------------- */
export const LAYERS = [
  { tag: "host", core: false, t: "VS Code · the playground · tests", d: "Owns the editor, the file watchers and the workspace scan. Speaks JSON strings to the core." },
  { tag: "wasm face", core: false, t: "RutLsp (rut-lsp-wasm)", d: "One exported class; every method a thin async delegation. The JS glue (envelopes, types) ships with the crate." },
  { tag: "actor", core: true, t: "Lsp → event loop → SessionState", d: "One inbox handle per session; one loop task owns all state and answers messages FIFO — no locks anywhere." },
  { tag: "queries", core: true, t: "hover · complete · definition · references · inlay · signature help · analyze", d: "Pure questions over the document and the mounted world. One module per request family." },
  { tag: "collection", core: true, t: "MountedFile · DefIndexCollection · Chain", d: "Every file in the session's world, one shape. Typed views over (doc, node); one home per cross-file question." },
  { tag: "semantic", core: true, t: "Document · classify · symbols · LineIndex", d: "The editor shell over the tree: UTF-16 positions, semantic-token coloring, the symbol tree — lazy." },
  { tag: "pipeline", core: true, t: "rut-ast Doc · rut-lexer Edit/Source", d: "Text + tokens + tree, one mutator (Doc::edit). An edit repairs the pipeline incrementally; queries never re-parse." },
];

export const DOORS = [
  ["type_decl(\"Circle\")", "the mounted-over-embed preference — a real mounted dep outranks the ambient prelude answer"],
  ["functions(\"length\")", "0, 1 or many — a Vec, the honesty rule; filter by owner() for free fns"],
  ["module_let(\"PI\")", "unique-or-none — ambiguity is a miss"],
  ["member(pos, node)", "THE member door (next section) — hover, definition and completion share it"],
  ["members · use_exports · bare", "completion's tier doors — enumeration and gates inside"],
  ["prefer_bodies(hits)", "body-over-decl — a package with both answers from the body"],
];

export const LANES = [
  { t: "provider queries", d: "analyze · hover · complete · definition · typeDefinition · inlay · references · signatureHelp · legend · forget — the editor's asks." },
  { t: "workspace-scan push", d: "indexFile · indexModAt · parseManifest — the host's findFiles respects editor-ignore semantics the env cannot." },
  { t: "dep pull", d: "setEnv + reloadDeps (deferred, coalesced under a 200 ms window) · resolveDeps (eager manual door). No entry mutates dep state except through the env." },
];

export const WIRE_JS = `// the wasm face — every method a thin delegation
const lsp = new RutLsp();
await lsp.setEnv(env);                    // the read-only HostEnv (below)

const out = await lsp.analyze(uri, src);
// → {"diags":[…],"tokens":{"data":[…]},"symbols":[…]}

const items = await lsp.complete(uri, src, 24, 5);  // takes src ALWAYS
// → [{ "label": "bump", "kind": 2, "detail": "fn bump(self) -> i32", … }]

await lsp.forget(uri);                    // drop a closed doc`;

export const HOSTENV_TS = `// the capability the host hands setEnv — read-only by construction
interface HostEnv {
  read(path: string): Uint8Array | null;     // miss = null, never throws
  readDir(path: string): string[] | null;    // null = no dir
  isDir(path: string): boolean;
  notify(message: string, isError: boolean); // the hint lane
  wakeAfter?(ms: number, wake: () => void);  // optional clock (setTimeout)
}`;

export const FILES = [
  ["src/lib.rs", "the public surface: Lsp (the inbox handle), every async RPC bridge"],
  ["src/env.rs", "Io — the read-only host capability + the optional clock"],
  ["src/event_loop.rs · src/state.rs", "the actor: FIFO loop, SessionState, the sync lane, the 200 ms defer window"],
  ["src/walk.rs · src/deps.rs · src/prelude.rs", "the read-only dep walk, manifest/lock tables, the ambient prelude"],
  ["src/semantic/", "Document (the shell) · classify + legend · symbols · UTF-16 LineIndex · mode_of"],
  ["src/hover/", "collection/ (mod.rs store · member.rs resolver · tiers.rs doors · views/) · bindings · lookup · infer · render"],
  ["src/query/", "one module per request family: analysis · completion · definition · references · inlay · signature_help · check"],
  ["src/wire.rs", "every envelope field name that is ours"],
  ["../rut-lsp-wasm/", "the wasm face: the exported RutLsp class + the crate-owned JS glue"],
  ["../rut-ast/ · ../rut-lexer/", "the pipeline: Doc, slot-map Ast, TreeIndex, Edit/Source, the token splice"],
];

export const LAWS = [
  ["Surface", "the crate is Lsp + env::Io; JSON strings on the wire — nothing else crosses."],
  ["Sync", "text arrives → sync (one incremental edit) → queries read. complete takes src always."],
  ["Artifact", "the tree is pure structure; text is a span × src join on the owning document."],
  ["Capsule", "nodes are POD; parse constructs, Doc::edit repairs; no field pokes anywhere."],
  ["Doors", "one home per question; views derive everything at read; nothing stored, nothing stale."],
  ["Tokens", "name_at is the fence residue, tokens_in the classifier's window — no other token reading."],
  ["Heuristics", "a miss is empty or null, never wrong text."],
];

export const GATES = [
  "cargo fmt --all --check · clippy (workspace + wasm32) · nextest — warnings are errors",
  "the extension lane: build:wasm → grammar corpus (123 files) + through-wasm e2e incl. the wire-shape block → vsce pack, byte-verifying the wasm inside",
  "edit ≡ cold property suites pin the incremental pipeline to the from-scratch parse",
];
