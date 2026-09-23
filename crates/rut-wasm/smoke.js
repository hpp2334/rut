// smoke: compile+run case1 through the wasm ABI (same glue the demo uses)
// + the StackTrace lane (err-channel phase 2): the SAME frame walk and
// lazy symbolication on wasm32 — a->b->c capture pinned innermost-first
// with exact call-site lines, through the raw ABI.
const fs = require("fs");
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
  const bytes = fs.readFileSync("target/wasm32-unknown-unknown/release/rut_wasm.wasm");
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const e = instance.exports;
  const enc = new TextEncoder();
  const dec = new TextDecoder();

  const src = enc.encode(`use ink::{ Logger };
enum Flavor { Sweet, Sour }
fn describe(f: Flavor) -> str {
    return when (f) {
        Flavor.Sweet -> "sweet",
        Flavor.Sour  -> "sour",
    };
}
pub fn main() -> nil {
    let name = "rut";
    let n = 41 + 1;
    let log = Logger.new("smoke");
    let sweet = "sweet";
    log.info(f"hi {name}! n={n} tab:\\t{sweet}");
    log.info(describe(Flavor.Sour));
}
`);
  const srcPtr = e.rut_alloc(src.length);
  new Uint8Array(e.memory.buffer, srcPtr, src.length).set(src);
  const resPtr = e.rut_compile(srcPtr, src.length);
  const resLen = new DataView(e.memory.buffer, resPtr, 4).getUint32(0, true);
  const resJson = dec.decode(new Uint8Array(e.memory.buffer, resPtr + 4, resLen));
  const compiled = JSON.parse(resJson);
  console.log("diags:", compiled.diags.length);
  console.log("ast nodes:", compiled.ast ? countNodes(compiled.ast) : 0);
  console.log("irDump bytes:", compiled.irDump.length);
  console.log("binary b64 bytes:", compiled.binary ? compiled.binary.length : 0);

  const bin = Buffer.from(compiled.binary, "base64");
  const binPtr = e.rut_alloc(bin.length);
  new Uint8Array(e.memory.buffer, binPtr, bin.length).set(bin);
  const runPtr = e.rut_run(binPtr, bin.length, 1000000n, 4n * 1024n * 1024n);
  const runLen = new DataView(e.memory.buffer, runPtr, 4).getUint32(0, true);
  const runJson = dec.decode(new Uint8Array(e.memory.buffer, runPtr + 4, runLen));
  console.log("run:", runJson);
})();

// ---- the StackTrace lane (err-channel phase 2): the same walk on wasm32 ----
// a -> b -> c captures; each body is >24 statements so the checker
// inliner keeps them as REAL frames (the host-lane driver pins use the
// same padding). The render rides ink::log through the wasm output
// channel — the raw ABI's one rut-visible output — and the exact RFC
// 0036 text is pinned here, byte for byte, on the wasm32 build.
(async () => {
  const bytes = fs.readFileSync("target/wasm32-unknown-unknown/release/rut_wasm.wasm");
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const e = instance.exports;
  const enc = new TextEncoder();
  const dec = new TextDecoder();

  function compileAndRun(src, fuel, heap) {
    const srcPtr = e.rut_alloc(src.length);
    new Uint8Array(e.memory.buffer, srcPtr, src.length).set(enc.encode(src));
    const resPtr = e.rut_compile(srcPtr, src.length);
    const resLen = new DataView(e.memory.buffer, resPtr, 4).getUint32(0, true);
    const compiled = JSON.parse(dec.decode(new Uint8Array(e.memory.buffer, resPtr + 4, resLen)));
    if (!compiled.binary) throw new Error("wasm lane compile failed: " + JSON.stringify(compiled.diags));
    const bin = Buffer.from(compiled.binary, "base64");
    const binPtr = e.rut_alloc(bin.length);
    new Uint8Array(e.memory.buffer, binPtr, bin.length).set(bin);
    const runPtr = e.rut_run(binPtr, bin.length, fuel, heap);
    const runLen = new DataView(e.memory.buffer, runPtr, 4).getUint32(0, true);
    return JSON.parse(dec.decode(new Uint8Array(e.memory.buffer, runPtr + 4, runLen)));
  }

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
pub fn main() -> nil {
    let r = a_big();
    let log = Logger.new("trace");
    log.info(r);
}
`;
  const env = compileAndRun(src, 1000000n, 4n * 1024n * 1024n);
  if (env.trap !== null) throw new Error("wasm trace lane trapped: " + env.trap);
  // Lines are relative to the SPLICED compilation unit: `ink` is an
  // explicitly-inlined module (graph.rs), so its 26 lines sit above the
  // user source — the capture lands at 28+26=54, and every call site
  // rides the same +26. Cols are the callee idents, exactly like the
  // host-lane pins. The frame walk and format are IDENTICAL to wasm32's
  // host twin; only the unit's line offset differs (splice, not trace).
  const expected = [
    "at c_big (rt:54:13)",     // the capture site: col 13
    "at b_big (rt:84:12)",     // the call that pushed the c_big frame
    "at a_big (rt:112:12)",
    "at main (rt:115:13)",
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
pub fn main() -> i32 {
    return grab();
}
`;
  const env2 = compileAndRun(loud, 100000n, 4n * 1024n * 1024n);
  if (env2.trap !== "IndexOutOfBounds") {
    throw new Error("wasm out-of-range did not trap loud: " + JSON.stringify(env2));
  }
  console.log("wasm StackTrace lane: render byte-exact on wasm32,", expected.length, "frames innermost-first; out-of-range traps loud");
  console.log("wasm StackTrace lane: ALL CHECKS PASSED");
})();
