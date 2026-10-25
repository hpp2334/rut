// smoke: analyze / hover / complete through the wasm-bindgen glue —
// the same module the VS Code extension bundles. Run after
//   wasm-pack build crates/rut-lsp-wasm --target web --release
//   node crates/rut-lsp-wasm/smoke.js        (from the repo root)
const fs = require("fs");
const path = require("path");
const assert = require("node:assert");

const PKG = path.join(__dirname, "pkg");

(async () => {
  const bytes = fs.readFileSync(path.join(PKG, "rut_lsp_wasm_bg.wasm"));
  const glue = await import(
    "file://" + path.join(PKG, "rut_lsp_wasm.js").replaceAll("\\", "/")
  );
  await glue.default({ module_or_path: bytes });
  const core = new glue.RutLsp();

  const URI = "file:///ws/main.rut";
  const SRC = `class Greeter {
    name: str;
}
impl Greeter {
    fn greet(self) -> str { return self.name; }
}
fn go(g: Greeter) -> str {
    return g.greet();
}
`;
  const lines = SRC.split("\n");

  // legend — names in token-type order
  const legend = JSON.parse(core.legend());
  assert.ok(legend.includes("keyword") && legend.includes("enumMember"), `legend: ${legend}`);

  // analyze a clean doc: no diags, a real token stream, nested symbols
  const a = JSON.parse(core.analyze(URI, SRC));
  assert.deepStrictEqual(a.diags, [], `diags: ${JSON.stringify(a.diags)}`);
  assert.ok(Array.isArray(a.tokens.data) && a.tokens.data.length >= 100, "token stream");
  assert.ok(a.symbols.some((sym) => sym.name === "Greeter"), "document symbols");
  assert.strictEqual(core.doc_len(URI), SRC.length, "doc round-trip");

  // hover over the `Greeter` type annotation
  const hover = JSON.parse(core.hover(URI, 6, lines[6].indexOf("Greeter")));
  assert.ok(hover && hover.contents.value.includes("Greeter"), "hover markdown");
  assert.ok(hover.range, "hover range");

  // member completion after `g.` — impl method + field
  const items = JSON.parse(core.complete(URI, 7, lines[7].indexOf("g.") + 2));
  const labels = items.map((i) => i.label);
  assert.ok(labels.includes("greet"), `methods complete: ${labels}`);
  assert.ok(labels.includes("name"), `fields complete: ${labels}`);

  // a workspace file's decls complete once the doc `use`s them — the
  // bare tier stays honest (own decls + ambient builtins + the names
  // the document already uses; a foreign plain offer the compile
  // would reject is a trap, so an unused foreign name never offers)
  core.add_def("file:///ws/gadgets/lib.rut", "class Widget {\n    id: i32;\n}\n");
  const honest = JSON.parse(core.complete(URI, 0, 0));
  assert.ok(!honest.map((i) => i.label).includes("Widget"), "bare tier stays honest without a use");
  JSON.parse(core.analyze(URI, "use gadgets::{ Widget };\n" + SRC));
  const bare = JSON.parse(core.complete(URI, 0, 0));
  assert.ok(bare.map((i) => i.label).includes("Widget"), "used workspace name completes");

  // a broken doc publishes rut-sourced errors
  const bad = JSON.parse(core.analyze("file:///ws/broken.rut", "fn broken(: nil {\n"));
  assert.ok(bad.diags.length > 0, "parse error surfaces");
  assert.strictEqual(bad.diags[0].source, "rut");
  assert.strictEqual(bad.diags[0].severity, 1);

  // the std surface rides inside the module: `len` resolves on str
  const STD_SRC = "fn f(s: str) -> i32 {\n    return s.len();\n}\n";
  const stdUri = "file:///ws/std.rut";
  JSON.parse(core.analyze(stdUri, STD_SRC));
  const stdHover = JSON.parse(core.hover(stdUri, 1, STD_SRC.split("\n")[1].indexOf("len") + 1));
  assert.ok(stdHover && stdHover.contents.value.includes("len"), "std surface hover");

  // references: the fn-local `g`'s decl -> its use in the body (the
  // definition index read backwards); includeDeclaration adds the decl
  const REF_SRC = "fn go(k: i32) -> i32 {\n    return k;\n}\n";
  const refUri = "file:///ws/refs.rut";
  JSON.parse(core.analyze(refUri, REF_SRC));
  const refs = JSON.parse(core.references(refUri, 0, 6, 0));
  assert.ok(Array.isArray(refs) && refs.length === 1, `param refs: ${JSON.stringify(refs)}`);
  assert.strictEqual(refs[0].range.start.line, 1, "the body use");
  const refsWithDecl = JSON.parse(core.references(refUri, 0, 6, 1));
  assert.strictEqual(refsWithDecl.length, 2, "includeDeclaration adds the decl ident");

  // signature help: the verbatim signature + the active slot
  const SIG_SRC = "fn hex_val(c: str, k: i32) -> i32 {\n    return k;\n}\nfn main() -> i32 {\n    return hex_val(\"a\", 2);\n}\n";
  const sigUri = "file:///ws/sig.rut";
  JSON.parse(core.analyze(sigUri, SIG_SRC));
  const sigLines = SIG_SRC.split("\n");
  const help = JSON.parse(core.signature_help(sigUri, 4, sigLines[4].indexOf("2);")));
  assert.ok(help && help.signatures.length === 1, `help: ${JSON.stringify(help)}`);
  assert.strictEqual(help.signatures[0].label, "fn hex_val(c: str, k: i32) -> i32");
  assert.strictEqual(help.signatures[0].parameters[1].label, "k: i32", "params as written");
  assert.strictEqual(help.activeParameter, 1, "the cursor sits on the second arg");

  // forget: a closed doc is gone (run last — it closes the main doc)
  core.forget(URI);
  assert.strictEqual(core.doc_len(URI), 0, "doc forgotten");

  console.log("smoke: ALL CHECKS PASSED");
})().catch((err) => {
  console.error(err);
  process.exit(1);
});
