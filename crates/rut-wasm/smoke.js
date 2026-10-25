// smoke: compile+run case1 through the wasm-bindgen glue (the same
// module the demo playground bundles) + the StackTrace lane (err-channel
// phase 2): the SAME frame walk and lazy symbolication on wasm32 —
// a->b->c capture pinned innermost-first with exact call-site lines,
// through the engine class. Run after
//   wasm-pack build crates/rut-wasm --target web --release
//   node crates/rut-wasm/smoke.js            (from the repo root)
const fs = require("fs");
const path = require("path");

const PKG = path.join(__dirname, "pkg");

function countNodes(n) {
  // structural walk over the flattened tagged AST objects
  let k = 1;
  for (const key of Object.keys(n)) {
    const v = n[key];
    if (v === null || typeof v !== "object") continue;
    if (Array.isArray(v)) {
      for (const x of v) {
        if (x && typeof x === "object" && typeof x.kind === "string") k += countNodes(x);
        else if (x && typeof x === "object" && x.node && typeof x.node.kind === "string") k += countNodes(x.node);
      }
    } else if (typeof v.kind === "string") {
      k += countNodes(v);
    }
  }
  return k;
}

(async () => {
  const bytes = fs.readFileSync(path.join(PKG, "rut_wasm_bg.wasm"));
  const glue = await import(
    "file://" + path.join(PKG, "rut_wasm.js").replaceAll("\\", "/")
  );
  await glue.default({ module_or_path: bytes });
  const engine = new glue.RutEngine();

  const compileAndRun = (src, fuel, heap) => {
    const compiled = JSON.parse(engine.compile(src));
    if (!compiled.binary) throw new Error("wasm lane compile failed: " + JSON.stringify(compiled.diags));
    const bin = Buffer.from(compiled.binary, "base64");
    return JSON.parse(engine.run(bin, fuel, heap));
  };

  // the demo's hello-format case (demo/src/cases.ts) — kept in sync by
  // hand here; the demo smoke is the corpus gate, this one the ABI's
  const compiled = JSON.parse(
    engine.compile(`use ink::{Logger};

enum Flavor { Sweet, Sour }

fn describe(f: Flavor) -> str {
    return when (f) {
        Flavor::Sweet -> "sweet",
        Flavor::Sour  -> "sour",
    };
}

entry fn main() {
    let log = Logger("case");
    let name = "rut";
    let n = 41 + 1;
    log.info(f"hi {name}! n={n} tab:\\t");
    log.info(describe(Flavor::Sour));
}
`)
  );
  if (compiled.diags.length) {
    throw new Error("smoke case must compile clean: " + JSON.stringify(compiled.diags));
  }
  console.log("diags:", compiled.diags.length);
  console.log("ast nodes:", compiled.ast ? countNodes(compiled.ast) : 0);
  console.log("irDump bytes:", compiled.irDump.length);
  console.log("binary b64 bytes:", compiled.binary ? compiled.binary.length : 0);

  const bin = Buffer.from(compiled.binary, "base64");
  const run = JSON.parse(engine.run(bin, 1000000n, 4n * 1024n * 1024n));
  console.log("run:", JSON.stringify(run));

  // ---- the StackTrace lane (err-channel phase 2): the same walk on wasm32 ----
  // a -> b -> c captures; each body is >24 statements so the checker
  // inliner keeps them as REAL frames (the host-lane driver pins use the
  // same padding). The render rides ink::log through the wasm output
  // channel — the engine's one rut-visible output — and the exact RFC
  // 0036 text is pinned here, byte for byte, on the wasm32 build.
  let pad = "";
  for (let i = 0; i < 25; i++) pad += `    let p${i} = ${i};\n`;
  const src = `use ink::{ Logger };
fn c_big() -> str {
${pad}    let t = capture_stacktrace();
    let r = t.render();
    return r;
}
fn b_big() -> str {
${pad}    return c_big();
}
fn a_big() -> str {
${pad}    return b_big();
}
entry fn main() -> nil {
    let r = a_big();
    let log = Logger("trace");
    log.info(r);
}
`;
  const env = compileAndRun(src, 1000000n, 4n * 1024n * 1024n);
  if (env.trap !== null) throw new Error("wasm trace lane trapped: " + env.trap);
  // Lines are relative to the SPLICED compilation unit: the host
  // surface (ink_host) rides above the user source, and every call
  // site's column is the callee ident, exactly like the host-lane
  // pins. The frame walk and format are IDENTICAL to wasm32's host
  // twin; the unit's file name and line offset are pinned byte-exact.
  const expected = [
    "at c_big (ink_host:28:13)",  // the capture site: col 13
    "at b_big (ink_host:58:12)",  // the call that pushed the c_big frame
    "at a_big (ink_host:86:12)",
    "at main (ink_host:89:13)",
  ];
  const got = env.output.length === 1 ? env.output[0].split("\n") : env.output;
  if (JSON.stringify(got) !== JSON.stringify(expected)) {
    throw new Error("wasm render mismatch:\n  got:      " + JSON.stringify(got) + "\n  expected: " + JSON.stringify(expected));
  }

  // the loud boundary, same on wasm32: out-of-range index traps
  const loud = `fn grab() -> i32 {
    let t = capture_stacktrace();
    return t.name(9).len();
}
entry fn main() -> i32 {
    return grab();
}
`;
  const env2 = compileAndRun(loud, 100000n, 4n * 1024n * 1024n);
  if (env2.trap !== "IndexOutOfBounds") {
    throw new Error("wasm out-of-range did not trap loud: " + JSON.stringify(env2));
  }
  console.log("wasm StackTrace lane: render byte-exact on wasm32,", expected.length, "frames innermost-first; out-of-range traps loud");
  console.log("wasm StackTrace lane: ALL CHECKS PASSED");
})().catch((err) => {
  console.error(err);
  process.exit(1);
});
