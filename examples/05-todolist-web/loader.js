// loader.js — the page shell's boot (the survey §3 shape: ~20 lines,
// because every DOM move happens INSIDE the module through web-sys —
// the JS side only hands over the app source and gets out of the way).
//
// The module is a cdylib with raw exports (the rut-wasm ABI pattern):
//   rut_web_alloc(len) -> ptr
//   rut_web_boot(ptr, len) -> i32       0 | -1
//   rut_web_last_error() -> ptr         [u32 le length][bytes], once
// DOM callbacks and timers pump the rut turn loop inside the module —
// the host owns the loop; JS never schedules anything.
//
// HONEST LIMITATION (survey §5.4): the module uses web-sys, which rides
// wasm-bindgen's imports. A browser run needs the generated glue, a
// manual build step this batch does not fake:
//
//   cargo build -p todolist-web --target wasm32-unknown-unknown --release
//   wasm-bindgen --target web --out-dir gen --out-name web_host \
//     ../../target/wasm32-unknown-unknown/release/todolist_web.wasm
//
// (`--out-name web_host` is the load-bearing part: this file imports
// ./gen/web_host.js, and the glue fetches ./web_host_bg.wasm relative
// to ITSELF — the subdir pair stays consistent.) gen/ is gitignored,
// so a build never dirties the tree.
// Without the glue this loader fails LOUD below with exactly that
// command — the repo's loud-fail law, never a silent skip. `node
// tests/e2e-browser.mjs` is the automated gate over the same steps.

const glueUrl = new URL("./gen/web_host.js", import.meta.url);

let glue;
try {
  glue = await import(glueUrl.href);
} catch (err) {
  throw new Error(
    "05-todolist-web: the wasm-bindgen glue (./gen/web_host.js) is missing — " +
      "a browser run is a manual build step:\n" +
      "  cargo build -p todolist-web --target wasm32-unknown-unknown --release\n" +
      "  wasm-bindgen --target web --out-dir gen --out-name web_host " +
      "../../target/wasm32-unknown-unknown/release/todolist_web.wasm\n" +
      `(original error: ${err})`,
  );
}

await glue.default(); // instantiate the module (the glue's init)

const w = glue.initSync();
const enc = new TextEncoder();
const source = enc.encode(await (await fetch(new URL("./rut/app/app/app.rut", import.meta.url))).text());

const ptr = w.rut_web_alloc(source.length);
new Uint8Array(w.memory.buffer, ptr, source.length).set(source);
if (w.rut_web_boot(ptr, source.length) !== 0) {
  const errPtr = w.rut_web_last_error();
  const len = new DataView(w.memory.buffer).getUint32(errPtr, true);
  const msg = new TextDecoder().decode(new Uint8Array(w.memory.buffer, errPtr + 4, len));
  throw new Error(`05-todolist-web: boot failed: ${msg}`);
}
