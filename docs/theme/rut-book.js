/**
 * rut-book.js — the book's ▶ Run buttons.
 *
 * Every ```rut fenced block whose text contains `pub fn main` gets a
 * "▶ Run" button; clicking it compiles + runs the block in-browser on
 * the rut wasm engine (crates/rut-wasm, the same artifact the demo
 * playground ships) and renders the program's output lines, compile
 * diagnostics, or — when the artifact is missing/invalid — a LOUD
 * panel naming the exact build command. There is no silent fallback:
 * a run either answers with the engine's envelope or says, loudly,
 * how to build the artifact.
 *
 * Runnable blocks are also EDITABLE: the code itself is
 * contenteditable (plaintext-only where supported, with plain-text
 * paste + literal-newline Enter guards where not), ▶ Run always
 * compiles the text as it stands on click, and a "↺" control at the
 * block's top-left restores the book's original code. An "edited"
 * badge marks blocks that no longer match the book's text.
 *
 * Plain browser JS, zero dependencies. The ABI is the wasm module's
 * export surface (mirrored by demo/src/wasm/rut-api.d.ts):
 *   memory, rut_alloc(len) -> ptr,
 *   rut_compile(src_ptr, src_len) -> ptr,
 *   rut_run(bin_ptr, bin_len, fuel: u64, heap: u64) -> ptr,
 *   rut_resume(extra_fuel, heap), rut_drop_frame()  (validated, unused
 *   here — a fresh rut_run supersedes any parked frame by itself)
 * Every call returns [u32 LE length][JSON bytes] at the returned ptr;
 * the compile envelope carries the binary base64-encoded. The engine
 * mounts the default packages (ink, pouch, json, strbuild, ...) the
 * same way `rut run` does — book blocks need no package wiring.
 *
 * The artifact is fetched from wasm/rut.wasm at the BOOK ROOT (not
 * committed; scripts/deploy.cjs --book builds and copies it, and
 * docs/book.toml ships it into the rendered book). The root-relative
 * URL comes from this script tag's own src — mdbook emits additional-js
 * with the page's correct "../theme/..." prefix, and this file always
 * lives one level under the root in theme/.
 */
(function () {
  "use strict";

  // the mechanism default is OFF (engine Limits::default() is uncapped);
  // THIS host explicitly opts into 10M fuel — freeze protection for the
  // editable blocks (a reader-typed infinite loop must trap, not hang
  // the tab), with no resume affordance here. The heap default (4 MiB)
  // mirrors the playground's.
  var DEFAULT_FUEL = 10000000;
  var DEFAULT_HEAP_BYTES = 4 * 1024 * 1024;

  // the exact commands every loud panel names (the repo's
  // no-silent-fallback law — the fix is always spelled out)
  var BUILD_CMD =
    "cargo build -p rut-wasm --target wasm32-unknown-unknown --release";
  var COPY_CMD =
    "cp target/wasm32-unknown-unknown/release/rut_wasm.wasm docs/wasm/rut.wasm";

  // ---- the module singleton (loaded lazily on first click) ------------

  var modulePromise = null; // the one WebAssembly.Instance, shared
  var artifactError = null; // set once the artifact proves missing/invalid

  /** book-root wasm/rut.wasm, derived from this script's own URL */
  function artifactUrl() {
    var src = null;
    if (document.currentScript && document.currentScript.src) {
      src = document.currentScript.src;
    } else {
      var el = document.querySelector('script[src*="rut-book"]');
      if (el) src = el.src;
    }
    if (src) return new URL("../wasm/rut.wasm", src).href;
    // last resort: correct only for root-level pages — never silent,
    // the fetch failure surfaces in the loud panel with the fix
    return "wasm/rut.wasm";
  }

  function fetchBytes(url) {
    return fetch(url).then(function (res) {
      if (!res.ok) throw new Error("fetch " + url + ": HTTP " + res.status);
      return res.arrayBuffer();
    });
  }

  /** instantiate: streaming when the server speaks wasm MIME, else bytes */
  function instantiate(url) {
    var bytes = fetchBytes(url);
    var streaming = WebAssembly.instantiateStreaming
      ? WebAssembly.instantiateStreaming(fetch(url), {})
      : Promise.reject(new Error("no instantiateStreaming"));
    return streaming.catch(function () {
      return bytes.then(function (buf) {
        return WebAssembly.instantiate(buf, {});
      });
    });
  }

  /** the export surface is the contract — an invalid artifact is LOUD */
  function validate(exports) {
    var fns = [
      "rut_alloc",
      "rut_compile",
      "rut_run",
      "rut_resume",
      "rut_drop_frame",
    ];
    for (var i = 0; i < fns.length; i += 1) {
      if (typeof exports[fns[i]] !== "function") {
        throw new Error("invalid rut.wasm: missing export " + fns[i]);
      }
    }
    if (!exports.memory) {
      throw new Error("invalid rut.wasm: missing export memory");
    }
  }

  function loadModule() {
    if (artifactError) return Promise.reject(new Error(artifactError));
    if (modulePromise) return modulePromise;
    var url = artifactUrl();
    modulePromise = instantiate(url)
      .then(function (wired) {
        var inst = wired.instance || wired;
        validate(inst.exports);
        return inst;
      })
      .catch(function (err) {
        // poison the singleton: every button (this page's and later
        // clicks') shows the loud panel; a retry re-fetches
        artifactError =
          (err && err.message ? err.message : String(err)) +
          " — fetched " +
          url;
        modulePromise = null;
        throw new Error(artifactError);
      });
    return modulePromise;
  }

  // ---- the raw ABI over linear memory ---------------------------------

  function writeBytes(exports, bytes) {
    var ptr = exports.rut_alloc(bytes.length);
    if (!ptr) throw new Error("wasm heap exhausted");
    new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
    return ptr;
  }

  /** [u32 LE length][JSON bytes] at the returned ptr */
  function readEnvelope(exports, ptr) {
    var len = new DataView(exports.memory.buffer, ptr, 4).getUint32(0, true);
    var bytes = new Uint8Array(exports.memory.buffer, ptr + 4, len);
    return new TextDecoder().decode(bytes);
  }

  function compileSrc(exports, src) {
    var bytes = new TextEncoder().encode(src);
    var ptr = writeBytes(exports, bytes);
    return JSON.parse(readEnvelope(exports, exports.rut_compile(ptr, bytes.length)));
  }

  function decodeBase64(b64) {
    var bin = atob(b64);
    var out = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i += 1) out[i] = bin.charCodeAt(i);
    return out;
  }

  function runBinary(exports, binary) {
    var bytes = decodeBase64(binary);
    var ptr = writeBytes(exports, bytes);
    return JSON.parse(
      readEnvelope(
        exports,
        exports.rut_run(ptr, bytes.length, BigInt(DEFAULT_FUEL), BigInt(DEFAULT_HEAP_BYTES)),
      ),
    );
  }

  // ---- DOM -----------------------------------------------------------

  /** the loud panel: the artifact is missing/invalid — name the fix */
  function loudPanel(out, why) {
    out.textContent = "";
    var panel = document.createElement("div");
    panel.className = "rut-run-loud";
    var head = document.createElement("div");
    head.className = "rut-run-loud-head";
    head.textContent = "rut.wasm is missing or invalid — the run buttons cannot work";
    var whyLine = document.createElement("div");
    whyLine.className = "rut-run-loud-why";
    whyLine.textContent = why || "";
    var fix = document.createElement("div");
    fix.className = "rut-run-loud-fix";
    fix.textContent = "Fix — build the artifact and copy it into docs/wasm/:";
    var cmd = document.createElement("code");
    cmd.textContent = BUILD_CMD + " && " + COPY_CMD;
    panel.appendChild(head);
    panel.appendChild(whyLine);
    panel.appendChild(fix);
    panel.appendChild(cmd);
    out.appendChild(panel);
  }

  function line(out, cls, text) {
    var el = document.createElement("div");
    el.className = cls;
    el.textContent = text; // verbatim — never interpreted
    out.appendChild(el);
    return el;
  }

  function renderRun(instance, out, src) {
    var exports = instance.exports;
    var compiled = compileSrc(exports, src);

    // compile diagnostics first — the loud channel, the button stays usable
    if (compiled.diags && compiled.diags.length) {
      compiled.diags.forEach(function (d) {
        var msg = d.msg || "diagnostic";
        if (typeof d.start === "number" && typeof d.end === "number") {
          msg += "  ·  at " + d.start + "–" + d.end;
        }
        line(out, "rut-run-err", msg);
      });
      return;
    }
    if (!compiled.binary) {
      line(out, "rut-run-err", "the engine emitted no binary and no diagnostics");
      return;
    }

    var run = runBinary(exports, compiled.binary);
    (run.output || []).forEach(function (l) {
      line(out, "rut-run-line", l);
    });
    if (!run.output || run.output.length === 0) {
      line(out, "rut-run-muted", "(no output)");
    }
    if (run.trap) {
      line(out, "rut-run-err", run.trap);
    }
    if (run.err) {
      line(out, "rut-run-err", "err: " + run.err);
    }
    if (!run.trap && !run.err && typeof run.fuelUsed === "number") {
      line(out, "rut-run-fuel", "fuel: " + run.fuelUsed);
    }
  }

  function wireButton(btn, pre, out, getSrc) {
    var inFlight = false;
    btn.addEventListener("click", function () {
      if (inFlight) return;
      var src = getSrc();
      if (!src || !src.trim()) {
        out.hidden = false;
        out.textContent = "";
        line(out, "rut-run-err", "nothing to run — the block is empty");
        return;
      }
      inFlight = true;
      btn.disabled = true;
      out.hidden = false;
      out.classList.add("rut-run-busy");
      out.textContent = "";
      line(out, "rut-run-muted", "▶ running…");

      var fail = function (err) {
        out.textContent = "";
        var why = err && err.message ? err.message : String(err);
        if (artifactError) {
          // the artifact lane is broken — the LOUD panel, everywhere
          loudPanel(out, why);
          controls.forEach(function (c) {
            c.out.hidden = false;
            c.out.classList.remove("rut-run-busy");
            c.out.textContent = "";
            loudPanel(c.out, why);
          });
        } else {
          line(out, "rut-run-err", why);
        }
      };

      loadModule()
        .then(function (instance) {
          out.textContent = "";
          renderRun(instance, out, src);
        })
        .catch(fail)
        .then(function () {
          inFlight = false;
          btn.disabled = false;
          out.classList.remove("rut-run-busy");
        });
    });
  }

  // the page's wired controls — the loud-panel path paints all of them
  var controls = [];

  /** the live source of an editable block: rendered text, nbsp cleaned */
  function srcOf(code) {
    var t = code.innerText !== undefined ? code.innerText : code.textContent;
    return String(t).replace(/\u00a0/g, " ").replace(/\r/g, "");
  }

  /**
   * Make a runnable block's code editable. plaintext-only is the clean
   * mode (plain-text paste, literal \n on Enter, no rich DOM); where the
   * browser does not support it we fall back to contenteditable=true and
   * enforce the same two invariants by hand. Every input re-marks the
   * block edited/unedited against the book's original text.
   */
  function makeEditable(code, pre, originalText) {
    var plain = false;
    try {
      code.contentEditable = "plaintext-only";
      plain = code.contentEditable === "plaintext-only";
    } catch (e) {
      plain = false;
    }
    if (!plain) {
      code.contentEditable = "true";
      code.addEventListener("paste", function (ev) {
        ev.preventDefault();
        var text = ev.clipboardData ? ev.clipboardData.getData("text/plain") : "";
        document.execCommand("insertText", false, text);
      });
      code.addEventListener("keydown", function (ev) {
        if (ev.key === "Enter") {
          ev.preventDefault();
          document.execCommand("insertText", false, "\n");
        }
      });
    }
    code.spellcheck = false;
    code.addEventListener("input", function () {
      pre.classList.toggle("rut-run-edited", srcOf(code) !== originalText);
    });
  }

  function makeRunBlock(code) {
    var pre = code.parentElement;
    if (!pre || pre.tagName !== "PRE") return;
    var src = code.textContent;
    if (src.indexOf("pub fn main") === -1) return; // not a whole program

    pre.classList.add("rut-run");

    var originalHtml = code.innerHTML;
    var originalText = srcOf(code);
    makeEditable(code, pre, originalText);

    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "rut-run-btn";
    btn.textContent = "▶ Run";
    pre.appendChild(btn);

    // ↺ restores the book's original code (innerHTML brings the baked
    // highlight spans back); the edited badge leaves with it
    var reset = document.createElement("button");
    reset.type = "button";
    reset.className = "rut-run-reset";
    reset.title = "restore the book's original code";
    reset.textContent = "↺";
    reset.addEventListener("click", function () {
      code.innerHTML = originalHtml;
      pre.classList.remove("rut-run-edited");
    });
    pre.appendChild(reset);

    var out = document.createElement("div");
    out.className = "rut-run-out";
    out.hidden = true;
    pre.insertAdjacentElement("afterend", out);

    controls.push({ btn: btn, out: out });
    wireButton(btn, pre, out, function () {
      return srcOf(code);
    });
  }

  function init() {
    var blocks = document.querySelectorAll("pre > code.language-rut");
    for (var i = 0; i < blocks.length; i += 1) makeRunBlock(blocks[i]);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
