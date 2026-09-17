// smoke: compile+run case1 through the wasm ABI (same glue the demo uses)
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

  const src = enc.encode(`
enum Flavor { Sweet, Sour }
fn describe(f: Flavor): string {
    return when (f) {
        Flavor.Sweet -> "sweet",
        Flavor.Sour  -> "sour",
    };
}
pub fn main(): nil {
    let name = "rut";
    let n = 41 + 1;
    print(f"hi {name}! n={n} tab:\\t'c'={'c'}");
    print(describe(Flavor.Sour));
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
