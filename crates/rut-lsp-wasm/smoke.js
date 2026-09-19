// smoke: analyze / hover / complete through the wasm ABI — the same glue
// the VS Code extension uses. Run after
//   cargo build -p rut-lsp-wasm --target wasm32-unknown-unknown --release
//   node smoke.js
const fs = require("fs");
const assert = require("node:assert");

(async () => {
  const bytes = fs.readFileSync("target/wasm32-unknown-unknown/release/rut_lsp_wasm.wasm");
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const e = instance.exports;
  const enc = new TextEncoder();
  const dec = new TextDecoder();

  // request protocol: rut_begin -> alloc+write inputs -> call -> read
  // (every (ptr, len) pair must be spread — `f(...put(a), put(b))` passes
  // put(b)'s ARRAY as one arg and reads garbage on the wasm ABI)
  const put = (s) => {
    const b = enc.encode(s);
    const p = e.rut_alloc(b.length);
    assert.notStrictEqual(p, 0, "arena overflow");
    new Uint8Array(e.memory.buffer, p, b.length).set(b);
    return [p, b.length];
  };
  const take = (ptr) => {
    assert.notStrictEqual(ptr, 0, "arena overflow");
    const len = new DataView(e.memory.buffer, ptr, 4).getUint32(0, true);
    return JSON.parse(dec.decode(new Uint8Array(e.memory.buffer, ptr + 4, len)));
  };

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
  e.rut_begin();
  const legend = take(e.rut_legend());
  assert.ok(legend.includes("keyword") && legend.includes("enumMember"), `legend: ${legend}`);

  // analyze a clean doc: no diags, a real token stream, nested symbols
  e.rut_begin();
  const [u, ul] = put(URI);
  const [s, sl] = put(SRC);
  const a = take(e.rut_analyze(u, ul, s, sl));
  assert.deepStrictEqual(a.diags, [], `diags: ${JSON.stringify(a.diags)}`);
  assert.ok(Array.isArray(a.tokens.data) && a.tokens.data.length >= 100, "token stream");
  assert.ok(a.symbols.some((sym) => sym.name === "Greeter"), "document symbols");
  assert.strictEqual(e.rut_doc_len(...put(URI)), SRC.length, "doc round-trip");

  // hover over the `Greeter` type annotation
  e.rut_begin();
  const hover = take(e.rut_hover(...put(URI), 6, lines[6].indexOf("Greeter")));
  assert.ok(hover && hover.contents.value.includes("Greeter"), "hover markdown");
  assert.ok(hover.range, "hover range");

  // member completion after `g.` — impl method + field
  e.rut_begin();
  const items = take(e.rut_complete(...put(URI), 7, lines[7].indexOf("g.") + 2));
  const labels = items.map((i) => i.label);
  assert.ok(labels.includes("greet"), `methods complete: ${labels}`);
  assert.ok(labels.includes("name"), `fields complete: ${labels}`);

  // a workspace file's decls complete bare (rut_add_def = the host's fs walk)
  e.rut_begin();
  e.rut_add_def(...put("file:///ws/lib.rut"), ...put("class Widget {\n    id: i32;\n}\n"));
  const bare = take(e.rut_complete(...put(URI), 0, 0));
  assert.ok(bare.map((i) => i.label).includes("Widget"), "workspace def completes");

  // a broken doc publishes rut-sourced errors
  e.rut_begin();
  const bad = take(e.rut_analyze(...put("file:///ws/broken.rut"), ...put("fn broken(: nil {\n")));
  assert.ok(bad.diags.length > 0, "parse error surfaces");
  assert.strictEqual(bad.diags[0].source, "rut");
  assert.strictEqual(bad.diags[0].severity, 1);

  // the std surface rides inside the module: `len` resolves on str
  const STD_SRC = "fn f(s: str) -> i32 {\n    return s.len();\n}\n";
  e.rut_begin();
  const stdUri = "file:///ws/std.rut";
  take(e.rut_analyze(...put(stdUri), ...put(STD_SRC)));
  e.rut_begin();
  const stdHover = take(e.rut_hover(...put(stdUri), 1, STD_SRC.split("\n")[1].indexOf("len") + 1));
  assert.ok(stdHover && stdHover.contents.value.includes("len"), "std surface hover");

  // forget: a closed doc is gone (run last — it closes the main doc)
  e.rut_begin();
  e.rut_forget(...put(URI));
  assert.strictEqual(e.rut_doc_len(...put(URI)), 0, "doc forgotten");

  console.log("smoke: ALL CHECKS PASSED");
})().catch((err) => {
  console.error(err);
  process.exit(1);
});
