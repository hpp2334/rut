#!/usr/bin/env node
// e2e-browser.mjs — the 05-todolist-web through-the-artifact gate.
//
// Builds nothing silently and skips nothing silently. Two tiers:
//
//   tier 1 (always, plain node): the REAL wasm artifact + the REAL
//     wasm-bindgen glue, driven through the loader's own ABI sequence
//     (import glue -> default() -> initSync() -> alloc -> write ->
//     boot -> last_error) on a ~60-line fake DOM — the full scripted
//     session the twin's tests run, headless.
//
//   tier 2 (when geckodriver + a browser exist): the REAL page —
//     glue generated into gen/ exactly as the manual recipe says,
//     the dir served over http, Firefox headless via raw
//     WebDriver HTTP (no npm deps), the same session typed and
//     clicked on the live DOM. gen/ is gitignored, so nothing is
//     deleted afterwards: the committed tree stays clean AND the
//     glue is left in place so the page keeps working locally.
//
// The session (`runSession`) is byte-stable across phases: the read/
// act ADAPTERS are what each DOM shape customizes. The page now
// paints the t1 framework's LOWERED DOM, so the adapters translate
// it at the selectors only:
//
//   * a committed row is `div.t1-row` under `#list` (hook `row-<id>`),
//     children 0=check 1=title 2=del as before;
//   * a check's state is the class TOKEN (`t1-check--on/--off` on the
//     framework-owned `button[role=checkbox]`), not `[x]` text — the
//     adapters map the token back to `[x]`/`[ ]` so the session's
//     assertions keep reading as they always have;
//   * a pending line (and a row's in-flight title — the same variant)
//     is `span.t1-text--pending`, not `class=pending`.
//
// Missing artifact/CLI  -> exit 1 with the exact build command
//                         (loud-fail: the gate guards the artifact).
// Missing geckodriver   -> tier 2 skipped LOUDLY with the manual
//                         recipe; tier 1 still gates (documented
//                         skip, exit 0 — the `test:host` convention).
//
// Usage: node tests/e2e-browser.mjs [--node-only]

import { spawn, spawnSync } from "node:child_process";
import net from "node:net";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const EXAMPLE = path.dirname(HERE); // examples/05-todolist-web
const ROOT = path.resolve(EXAMPLE, "..", "..");
const WASM = path.join(ROOT, "target", "wasm32-unknown-unknown", "release", "todolist_web.wasm");
const APP_SRC = path.join(EXAMPLE, "todolist.rut");
const BINDGEN_VERSION = "0.2.128"; // must match Cargo.lock's wasm-bindgen
const NODE_ONLY = process.argv.includes("--node-only");

// ---- the loud helpers ----

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let failures = 0;
function expect(cond, what) {
  if (cond) {
    console.log(`  ok: ${what}`);
  } else {
    failures += 1;
    console.error(`  FAIL: ${what}`);
  }
}

async function waitFor(fn, what, timeoutMs = 6000) {
  const deadline = Date.now() + timeoutMs;
  let last;
  for (;;) {
    last = await fn();
    if (last) { expect(true, what); return; }
    if (Date.now() > deadline) {
      expect(false, `${what} (timed out; last: ${JSON.stringify(last)})`);
      return;
    }
    await sleep(50);
  }
}

function loudFail(msg) {
  console.error(`e2e-browser: ${msg}`);
  process.exit(1);
}

// ---- tier 0: the artifact + the glue CLI ----

function ensureArtifact() {
  if (fs.existsSync(WASM)) return;
  console.log("artifact missing — building (the manual recipe's first line):");
  console.log("  cargo build -p todolist-web --target wasm32-unknown-unknown --release");
  const r = spawnSync("cargo", ["build", "-p", "todolist-web", "--target", "wasm32-unknown-unknown", "--release"], {
    cwd: ROOT, stdio: "inherit",
  });
  if (r.status !== 0 || !fs.existsSync(WASM)) {
    loudFail(
      "the wasm artifact could not be produced — run it yourself:\n" +
      "  cargo build -p todolist-web --target wasm32-unknown-unknown --release",
    );
  }
}

function findBindgen() {
  const candidates = [];
  if (process.env.WASM_BINDGEN) candidates.push(process.env.WASM_BINDGEN);
  for (const dir of (process.env.PATH ?? "").split(path.delimiter)) {
    const p = path.join(dir, "wasm-bindgen");
    if (fs.existsSync(p)) candidates.push(p);
  }
  // wasm-pack's fetched installs — the CLI version MUST match the
  // crate's wasm-bindgen (Cargo.lock), or the glue and the module
  // disagree at instantiate time
  const cache = path.join(os.homedir(), ".cache", ".wasm-pack");
  if (fs.existsSync(cache)) {
    for (const entry of fs.readdirSync(cache)) {
      const p = path.join(cache, entry, "wasm-bindgen");
      if (fs.existsSync(p)) candidates.push(p);
    }
  }
  for (const cli of candidates) {
    const v = spawnSync(cli, ["--version"], { encoding: "utf8" });
    if (v.status === 0 && v.stdout.includes(BINDGEN_VERSION)) return cli;
  }
  loudFail(
    `wasm-bindgen CLI ${BINDGEN_VERSION} not found (checked PATH, WASM_BINDGEN, ` +
    "~/.cache/.wasm-pack) — install the version pinned in Cargo.lock:\n" +
    `  cargo install wasm-bindgen-cli --version ${BINDGEN_VERSION}`,
  );
}

function makeGlue(bindgen, outDir) {
  const r = spawnSync(bindgen, ["--target", "web", "--out-dir", outDir, "--out-name", "web_host", WASM], {
    cwd: ROOT, encoding: "utf8",
  });
  if (r.status !== 0) {
    loudFail(
      "wasm-bindgen failed:\n" + (r.stderr ?? r.stdout ?? "") +
      "\n(the manual recipe step: wasm-bindgen --target web --out-dir gen --out-name web_host " +
      "target/wasm32-unknown-unknown/release/todolist_web.wasm)",
    );
  }
  return path.join(outDir, "web_host.js");
}

// ---- the scripted session: the twin's story, told twice ----
// add x2 outstanding -> deadline-then-sequence commits; the client
// gate; deadline order ACROSS kinds (toggle 250 < add 400); remove.
// `read`/`act` are async in BOTH tiers so the session code is shared.

async function runSession(read, act) {
  // the boot shape: the shell built under the seeded #app, first paint
  await waitFor(() => read.status(), "the boot turn paints the status line");
  expect((await read.status()) === "0 open | 0 done | 0 in flight — booted — type a title, press Add",
    "boot paints the empty counts line");
  expect((await read.placeholder()) === "What needs doing?", "the field carries its placeholder");
  expect((await read.buttonText()) === "Add", "the add button is 'Add'");
  expect((await read.rows()).length === 0 && (await read.pending()).length === 0, "the list paints empty");

  // the client-side gate: an empty draft books nothing
  await act.clickAdd();
  expect((await read.status()).endsWith("type a title first"), "empty draft gated client-side");
  expect((await read.pending()).length === 0, "the gate booked no request");

  // add x2 outstanding: pending rows NOW, list unchanged
  await act.type("milk");
  await waitFor(async () => (await read.status()).includes("typing 'milk'"),
    "typing carries the value as the event row's detail");
  await act.clickAdd();
  expect((await read.status()).endsWith("requested add 'milk'"), "the add is a REQUEST");
  expect((await read.pending()).join("|") === "... milk", "the pending row paints now");
  expect((await read.fieldValue()) === "", "the field cleared: the draft moved into the request");
  await act.type("tea");
  await act.clickAdd();
  expect((await read.pending()).join("|") === "... milk|... tea", "two outstanding adds, book order");

  // the answers land in deadline-then-sequence order
  await waitFor(async () => (await read.status()).endsWith("added 'tea' as #2"), "the round trip completes");
  expect((await read.rows()).join("|") === "[ ] milk|[ ] tea", "commits in deadline-then-sequence order");
  expect((await read.pending()).length === 0, "no pending rows remain");

  // deadline order ACROSS kinds, on the REAL clock: a fast toggle
  // (250) booked before a slow add (400) — the toggle must commit
  // first, visibly. (The twin's stronger shape — the SLOW request
  // booked FIRST and still committing second — needs the virtual
  // clock: a WebDriver click can take longer than the 150ms latency
  // gap, so here the booking order itself carries the guarantee.)
  let sawFlight = false;
  let last = { marks: [], pending: [] };
  const watch = async () => {
    for (let t = 0; t < 100; t += 1) {
      // ONE atomic snapshot per sample: two separate reads can straddle
      // the flight window (the gap is only latency-difference wide)
      last = await read.snap();
      const [mark] = last.marks;
      const jamPending = last.pending.includes("... jam");
      if (mark === "[x]" && jamPending) { sawFlight = true; break; }
      if (mark === "[x]" && !jamPending) break; // both landed
      await sleep(40);
    }
  };
  await act.clickMark(1);
  await act.type("jam");
  await act.clickAdd();
  await watch();
  expect(sawFlight, `deadline order across kinds: toggle [x] commits while '... jam' still flies (last: ${JSON.stringify(last)})`);
  await waitFor(async () => (await read.status()).endsWith("added 'jam' as #3"), "the slow add lands last");
  const marks = await read.rowMarks();
  expect(marks[0] === "[x]" && marks[2] === "[ ]", "row marks after both commits");

  // the remove round trip (fastest kind)
  await act.clickDel(1);
  await waitFor(async () => (await read.status()).endsWith("removed 'milk' (#1)"), "the remove answers");
  expect((await read.rows()).join("|") === "[ ] tea|[ ] jam", "milk's row is gone, order preserved");
  expect((await read.status()).startsWith("2 open | 0 done | 0 in flight"), "the final counts line");
}

// ---- tier 1: the fake DOM + the loader's ABI sequence in node ----

function readEnvelope(w) {
  const p = w.rut_web_last_error();
  if (!p) return "";
  const len = new DataView(w.memory.buffer).getUint32(p, true);
  return new TextDecoder().decode(new Uint8Array(w.memory.buffer, p + 4, len));
}

async function tier1(gluePath) {
  console.log("\ntier 1 — the artifact + glue through the loader's ABI (plain node)");

  // The fake DOM: duck-typed for the glue's shims. The only hard laws
  // are the two `instanceof` checks (HTMLInputElement, Window), which
  // the fake globals satisfy. setAttribute("id") indexes by id — the
  // real DOM's own law, and the app's ui_get contract.
  const byId = new Map();
  class FakeElement {
    constructor(tag) {
      this.tagName = tag.toUpperCase();
      this.children = [];
      this.attrs = new Map();
      this.textContent = undefined;
      this.listeners = new Map();
      this.value = undefined;
    }
    get firstChild() { return this.children[0] ?? null; }
    appendChild(c) {
      // appendChild MOVES a child that is already live (the real DOM's
      // own law, modelled by the rust twin at fake_dom.rs too) — the
      // keyed diff's reorder path re-appends survivors, and a push
      // here would duplicate the node instead of moving it
      const at = this.children.indexOf(c);
      if (at >= 0) this.children.splice(at, 1);
      this.children.push(c);
      return c;
    }
    removeChild(c) {
      const at = this.children.indexOf(c);
      if (at < 0) throw new Error("NotFoundError");
      this.children.splice(at, 1);
      return c;
    }
    setAttribute(n, v) { this.attrs.set(n, v); if (n === "id") byId.set(v, this); }
    addEventListener(type, cb) {
      if (!this.listeners.has(type)) this.listeners.set(type, []);
      this.listeners.get(type).push(cb);
    }
    fire(type) { for (const cb of this.listeners.get(type) ?? []) cb(); }
  }
  class HTMLInputElement extends FakeElement {}
  class Window {}
  byId.set("app", new FakeElement("div"));
  globalThis.HTMLInputElement = HTMLInputElement;
  globalThis.Window = Window;
  Object.setPrototypeOf(globalThis, Window.prototype);
  globalThis.document = {
    getElementById: (id) => byId.get(id) ?? null,
    createElement: (tag) => (tag === "input" ? new HTMLInputElement(tag) : new FakeElement(tag)),
  };

  // the loader's exact sequence: import glue -> default() -> initSync()
  const glue = await import(pathToFileURL(gluePath).href);
  const bytes = fs.readFileSync(path.join(path.dirname(gluePath), "web_host_bg.wasm"));
  await glue.default({ module_or_path: new WebAssembly.Module(bytes) });
  const w = glue.initSync();

  // the ABI round trip BEFORE boot: exports + the error envelope
  expect(
    typeof w.rut_web_alloc === "function" && typeof w.rut_web_boot === "function"
    && typeof w.rut_web_pump === "function" && typeof w.rut_web_last_error === "function"
    && w.memory instanceof WebAssembly.Memory,
    "the four raw exports + memory are present",
  );
  expect(w.rut_web_pump() === -1, "pump before boot fails loud (-1)");
  expect(readEnvelope(w) === "pump before boot", "last_error carries the envelope message");
  expect(readEnvelope(w) === "", "last_error reads once, then null");

  // boot the REAL app source (the loader fetches it; we hand it over
  // through the same alloc/write/boot path)
  const src = fs.readFileSync(APP_SRC);
  const ptr = w.rut_web_alloc(src.length);
  new Uint8Array(w.memory.buffer, ptr, src.length).set(src);
  expect(w.rut_web_boot(ptr, src.length) === 0, "boot compiles, mounts, verifies, and runs main (rc 0)");

  // The lowered DOM's adapters: committed rows are the `t1-row`
  // children of #list; a pending line (or an in-flight row title) is
  // the `t1-text--pending` variant; a check's state is its class
  // token, mapped back to the `[x]`/`[ ]` the session has always
  // asserted. Child order inside a row is unchanged: 0=mark 1=title.
  const isRow = (c) => (c.attrs.get("class") ?? "").includes("t1-row");
  const isPendingLine = (c) => (c.attrs.get("class") ?? "").includes("t1-text--pending");
  const markOf = (row) => ((row.children[0].attrs.get("class") ?? "").includes("t1-check--on") ? "[x]" : "[ ]");
  const committed = () => byId.get("list").children.filter(isRow);
  const field = byId.get("new-todo");
  const read = {
    status: async () => byId.get("status").textContent ?? "",
    placeholder: async () => field.attrs.get("placeholder") ?? "",
    buttonText: async () => byId.get("add-btn").textContent ?? "",
    fieldValue: async () => field.value ?? "",
    rows: async () => committed().map((row) => `${markOf(row)} ${row.children[1].textContent}`),
    rowMarks: async () => committed().map(markOf),
    pending: async () => byId.get("list").children.filter(isPendingLine)
      .map((c) => c.textContent),
    // atomic by construction: these reads are local object reads
    snap: async () => ({
      marks: committed().map(markOf),
      pending: byId.get("list").children.filter(isPendingLine)
        .map((c) => c.textContent),
    }),
  };
  // the fire-and-flush shape: DOM fires pump synchronously; a small
  // sleep lets booked timers (node's own clock here) land when the
  // session polls — mirroring the browser's event-loop ownership
  const act = {
    type: async (text) => { field.value = text; field.fire("input"); await sleep(0); },
    clickAdd: async () => { byId.get("add-btn").fire("click"); await sleep(0); },
    clickMark: async (id) => { byId.get(`row-${id}`).children[0].fire("click"); await sleep(0); },
    clickDel: async (id) => { byId.get(`row-${id}`).children[2].fire("click"); await sleep(0); },
  };
  await runSession(read, act);
}

// ---- tier 2: the real page, Firefox headless over WebDriver ----

function haveBinary(name) {
  for (const dir of (process.env.PATH ?? "").split(path.delimiter)) {
    if (fs.existsSync(path.join(dir, name))) return true;
  }
  return false;
}

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
    srv.on("error", reject);
  });
}

function serveStatic(dir) {
  const mime = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".rut": "text/plain" };
  const srv = http.createServer((req, res) => {
    const url = new URL(req.url, "http://x");
    const file = path.join(dir, url.pathname);
    if (!file.startsWith(dir) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.writeHead(404).end("nope");
      return;
    }
    res.writeHead(200, { "content-type": mime[path.extname(file)] ?? "application/octet-stream" });
    res.end(fs.readFileSync(file));
  });
  return new Promise((resolve) => srv.listen(0, "127.0.0.1", () => resolve({ srv, port: srv.address().port })));
}

const EID = (e) => e["element-6066-11e4-a52e-4f735466cecf"] ?? e.ELEMENT;

async function tier2() {
  console.log("\ntier 2 — the real page (Firefox headless, geckodriver over WebDriver)");

  // the manual recipe, automated: glue INTO gen/ (the gitignored
  // output dir — the loader imports ./gen/web_host.js; the glue
  // fetches ./web_host_bg.wasm relative to ITSELF, so the subdir
  // pair stays consistent)
  const GEN = path.join(EXAMPLE, "gen");
  fs.mkdirSync(GEN, { recursive: true });
  makeGlue(findBindgen(), GEN);
  const { srv, port } = await serveStatic(EXAMPLE);
  const gdPort = await freePort();
  const gd = spawn("geckodriver", ["--port", String(gdPort)], { stdio: "ignore" });
  const base = `http://127.0.0.1:${gdPort}`;
  const req = async (method, p, body) => {
    const r = await fetch(base + p, {
      method,
      body: body === undefined ? undefined : JSON.stringify(body),
      headers: body === undefined ? {} : { "content-type": "application/json" },
    });
    const j = await r.json();
    if (j.value && j.value.error) throw new Error(`webdriver ${method} ${p}: ${j.value.error} — ${j.value.message}`);
    return j.value;
  };
  let sid = null;
  try {
    for (let i = 0; i < 50; i += 1) {
      try { await req("GET", "/status"); break; } catch { await sleep(100); }
    }
    const caps = {
      capabilities: {
        alwaysMatch: { browserName: "firefox", "moz:firefoxOptions": { args: ["-headless"] } },
      },
    };
    if (process.env.FIREFOX_BIN) caps.capabilities.alwaysMatch["moz:firefoxOptions"].binary = process.env.FIREFOX_BIN;
    sid = (await req("POST", "/session", caps)).sessionId;
    await req("POST", `/session/${sid}/url`, { url: `http://127.0.0.1:${port}/index.html` });

    const find = (sel) => req("POST", `/session/${sid}/element`, { using: "css selector", value: sel }).then(EID);
    const findAll = async (sel) => (await req("POST", `/session/${sid}/elements`, { using: "css selector", value: sel })).map(EID);
    // req() returns the wire value already: text -> string, a
    // missing attribute/property -> null
    const text = async (sel) => (await req("GET", `/session/${sid}/element/${await find(sel)}/text`)) ?? "";
    const attr = async (sel, name) => req("GET", `/session/${sid}/element/${await find(sel)}/attribute/${name}`);
    // the app sets the field's value via the JS property (set_value),
    // not the attribute — read the property endpoint
    const prop = async (sel, name) => req("GET", `/session/${sid}/element/${await find(sel)}/property/${name}`);
    const click = async (sel) => { await req("POST", `/session/${sid}/element/${await find(sel)}/click`, {}); };
    const keys = async (sel, t) => { await req("POST", `/session/${sid}/element/${await find(sel)}/value`, { text: t }); };

    const attrOfEl = (el, name) => req("GET", `/session/${sid}/element/${el}/attribute/${name}`);
    // The lowered DOM, translated at the selectors only: rows are
    // `div.t1-row` (hook `row-<id>`), the check is the framework's
    // `button[role=checkbox]` whose STATE is the `t1-check--on/--off`
    // token (mapped back to `[x]`/`[ ]` for the session), the title is
    // the row's one `span`, a pending line is `span.t1-text--pending`.
    // Child order 0=mark 1=title 2=del is unchanged.
    const rowIds = async () => {
      const rows = await findAll("#list .t1-row");
      const out = [];
      for (const row of rows) out.push(await attrOfEl(row, "id"));
      return out;
    };
    const markTokenToText = (cls) => ((cls ?? "").includes("t1-check--on") ? "[x]" : "[ ]");
    // reads ride the page's own event loop (the app rebuilds rows
    // between polls, the module boots after load) — a transient
    // no-such/stale element is a poll miss, not a failure
    const soft = (p) => p.catch((e) => {
      if (/no such|stale/.test(String(e))) return null;
      throw e;
    });
    const read = {
      status: async () => (await soft(text("#status"))) ?? "",
      placeholder: async () => (await soft(attr("#new-todo", "placeholder"))) ?? "",
      buttonText: async () => (await soft(text("#add-btn"))) ?? "",
      fieldValue: async () => (await soft(prop("#new-todo", "value"))) ?? "",
      rows: async () => {
        const out = [];
        for (const id of await rowIds()) {
          out.push(`${markTokenToText(await soft(attr(`#${id} [role="checkbox"]`, "class")))} ${await soft(text(`#${id} span`))}`);
        }
        return out;
      },
      rowMarks: async () => {
        const out = [];
        for (const id of await rowIds()) {
          out.push(markTokenToText(await soft(attr(`#${id} [role="checkbox"]`, "class"))));
        }
        return out;
      },
      pending: async () => {
        const out = [];
        for (const el of await findAll("#list span.t1-text--pending")) out.push((await soft(req("GET", `/session/${sid}/element/${el}/text`))) ?? "");
        return out;
      },
      // ONE in-page evaluation: both facts from a single instant,
      // immune to the read-between-reads race
      snap: async () => {
        const s = await req("POST", `/session/${sid}/execute/sync`, {
          script: "return JSON.stringify({ marks: [...document.querySelectorAll('#list .t1-row')].map((r) => r.children[0].classList.contains('t1-check--on') ? '[x]' : '[ ]'), pending: [...document.querySelectorAll('#list span.t1-text--pending')].map((l) => l.textContent) })",
          args: [],
        });
        return JSON.parse(s);
      },
    };
    const act = {
      type: (t) => keys("#new-todo", t),
      clickAdd: () => click("#add-btn"),
      clickMark: (id) => click(`#row-${id} [role="checkbox"]`),
      clickDel: (id) => click(`#row-${id} button.t1-btn--quiet`),
    };
    try {
      await runSession(read, act);
    } catch (e) {
      // the page's own state, for the log — then the failure stands
      try {
        const html = await req("POST", `/session/${sid}/execute/sync`, { script: "return document.body.innerHTML", args: [] });
        console.error(`  page at failure: ${String(html).replace(/\s+/g, " ").slice(0, 500)}`);
      } catch { /* session gone */ }
      throw e;
    }
  } finally {
    if (sid) { try { await req("DELETE", `/session/${sid}`); } catch { /* gone */ } }
    gd.kill();
    srv.close();
  }
}

// ---- main ----

ensureArtifact();
const bindgen = findBindgen();
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "todolist-web-e2e-"));
try {
  const gluePath = makeGlue(bindgen, tmp);
  await tier1(gluePath);
} finally {
  fs.rmSync(tmp, { recursive: true, force: true });
}

if (NODE_ONLY) {
  console.log("\ntier 2 skipped by --node-only");
} else if (haveBinary("geckodriver") && (haveBinary("firefox") || process.env.FIREFOX_BIN)) {
  await tier2();
} else {
  console.log(
    "\ntier 2 SKIPPED loudly — geckodriver/firefox not on PATH. The manual browser run:\n" +
    "  cd examples/05-todolist-web\n" +
    "  cargo build -p todolist-web --target wasm32-unknown-unknown --release\n" +
    `  wasm-bindgen --target web --out-dir gen --out-name web_host ${path.relative(EXAMPLE, WASM)}\n` +
    "  python3 -m http.server   # in that dir; then open http://localhost:8000/",
  );
}

console.log(`\ne2e-browser: ${failures === 0 ? "ALL CHECKS PASSED" : `${failures} CHECK(S) FAILED`}`);
process.exit(failures === 0 ? 0 : 1);
