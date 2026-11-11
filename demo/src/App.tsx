import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CASES, DEFAULT_BUDGET, ENABLED_FUEL_DEFAULT, type RutCase } from "./cases";
import { EXAMPLES } from "./examples";
import { BUILD_WASM_COMMAND, Runner } from "./runner";
import { CaseList, type CaseGroup } from "./components/CaseList";
import { DepsPanel, type DepRowView } from "./components/DepsPanel";
import { Editor, type Highlight } from "./components/Editor";
import { Panes, type PaneData } from "./components/Panes";
import { StatusBar } from "./components/StatusBar";
import { PLAYGROUND_DOC, RutLsp, decodeTokens, isEnvelope } from "./lsp/rut-lsp";
import {
  LOCK_PATH,
  MANIFEST_PATH,
  MemoryEnv,
  WORKSPACE_ROOT,
  renderLockText,
  renderManifestText,
  sha256Hex,
  type LockRow,
} from "./lsp/store-env";
import { STD_PINS } from "./lsp/std-pins";
import { buildOverlayCached, type OverlayCache } from "./lsp/overlay";
import type { CompileResult, Diag, RunResult } from "./wasm/rut-api";

const EMPTY_PANES: PaneData = { output: [], irDump: "" };
/** the while-typing lane (survey D3, tuned by the audit's M-1): ONE
 * debounced timer, two consumers — the overlay rebuild and the D-1
 * auto-run. 120 ms: the analyze is 0.1 ms at case sizes, the budget is
 * all human patience; the number is the catch-up record, not a gate. */
const ANALYZE_DEBOUNCE_MS = 120;
/** the D-1 auto-run stance: case switches and the edit lane run at the
 * host's explicit WATCHDOG slice — the mechanism default is OFF, but
 * `rut_run` is a synchronous main-thread wasm call, so an uncapped
 * auto-run on a stray long loop is a hard tab freeze. This slice is
 * the host's deliberate self-protection, NOT a user-facing default:
 * the user's own ▶ Run is uncapped unless they enable the fuel box. */
const AUTO_FUEL = 1_000_000;

/** the full-page boot-error panel — THE RUNNER LAW (survey D1):
 * wasm-or-error, the panes never mount without the artifact */
function BootError(props: { banner: string }): JSX.Element {
  return (
    <div className="boot-error">
      <div className="boot-error-box">
        <h1>rut.wasm missing or invalid</h1>
        <p>
          The playground only shows <strong>real runs</strong> — there is
          no preview fallback. Boot failed with:
        </p>
        <pre className="boot-error-detail">{props.banner}</pre>
        <p>To build the artifact, run this in the demo/ directory:</p>
        <pre className="boot-error-command">
          <code>{BUILD_WASM_COMMAND}</code>
        </pre>
        <p className="boot-error-note">
          (builds <code>crates/rut-wasm</code> for wasm32 and copies it to{" "}
          <code>public/rut.wasm</code>)
        </p>
      </div>
    </div>
  );
}

/** the highlight boot-error panel — THE BINDING LAW (survey D3):
 * highlight rides the analyzer or the page says so loudly; the panel
 * rides the phase-1 error-panel shape and names the exact command */
function LspError(props: { detail: string }): JSX.Element {
  return (
    <div className="boot-error">
      <div className="boot-error-box">
        <h1>rut-lsp.wasm missing or invalid</h1>
        <p>
          The playground&rsquo;s highlighting rides the analyzer&rsquo;s
          semantic tokens — there is no fake fallback. Boot failed with:
        </p>
        <pre className="boot-error-detail">{props.detail}</pre>
        <p>To build the artifact, run this in the demo/ directory:</p>
        <pre className="boot-error-command">
          <code>{BUILD_WASM_COMMAND}</code>
        </pre>
        <p className="boot-error-note">
          (builds <code>crates/rut-lsp-wasm</code> for wasm32 and copies
          it to <code>public/rut-lsp.wasm</code>)
        </p>
      </div>
    </div>
  );
}

/** the virtual world's initial declaration: the nine std zips as url
 * rows (the `rut new` scaffold shape — std rides the dep lane, nothing
 * ambient beyond `core`), pinned by the lock the walk reads */
function initialDepRows(): Array<{ name: string; url: string }> {
  return STD_PINS.map((p) => ({ name: p.name, url: p.url }));
}

function initialLockRows(): LockRow[] {
  return STD_PINS.map((p) => ({ pkg: p.name, url: p.url, sha256: p.sha256 }));
}

export function App(): JSX.Element {
  const groups: CaseGroup[] = useMemo(
    () => [
      { title: "cases", cases: CASES },
      { title: "classics", cases: EXAMPLES },
    ],
    [],
  );
  const [runner, setRunner] = useState<Runner | null>(null);
  const [currentCase, setCurrentCase] = useState<RutCase>(CASES[0]);
  const [source, setSource] = useState(CASES[0].source);
  const [panes, setPanes] = useState<PaneData>(EMPTY_PANES);
  // fuel is OFF by default: the box budget below is the REMEMBERED
  // enabled budget (seeded on first enable); the runner only receives
  // it when fuelEnabled is true — otherwise runs are uncapped (fuel 0)
  const [fuelEnabled, setFuelEnabled] = useState(false);
  const [budget, setBudget] = useState({
    ...DEFAULT_BUDGET,
    fuel: ENABLED_FUEL_DEFAULT,
  });
  const [fuelUsed, setFuelUsed] = useState(0);
  const [heapUsed, setHeapUsed] = useState(0);
  const [parked, setParked] = useState(false);
  const [running, setRunning] = useState(false);
  const runToken = useRef(0);
  /** the last successful compile — a Resume re-renders its AST/IR too */
  const lastCompiled = useRef<CompileResult | null>(null);
  /** the source that compile came from — the AST tree and its literal
   * slices ship as ONE version in the panes data (the M-1 boundary:
   * typing keeps the panes' identity stable until the next run) */
  const lastCompiledSrc = useRef("");
  // the highlight layer (survey D3): one binding, booted once; the
  // overlay + squiggles come from the debounced full rut_analyze
  const [lsp, setLsp] = useState<RutLsp | null>(null);
  const [lspMode, setLspMode] = useState<"booting" | "live" | "error">(
    "booting",
  );
  const [lspDetail, setLspDetail] = useState("");
  const [highlight, setHighlight] = useState<Highlight | null>(null);
  /** the world version — every landed walk bumps it, and the analyze
   * lane re-arms for the SAME source (a mounted dep changes the check
   * lane's answer; the run lane never re-runs on it) */
  const [worldTick, setWorldTick] = useState(0);
  /** the incremental overlay cache (the M-1 fix): untouched lines keep
   * their span-array identity across analyzes */
  const overlayCache = useRef<OverlayCache | undefined>(undefined);
  /** the source whose run was IMMEDIATE (a case switch — or the boot
   * install): the debounced lane skips that exact source, so a case
   * opening runs ONCE, not twice. Null = the lane is an EDIT lane. */
  const immediateRunSrc = useRef<string | null>(CASES[0].source);
  /** the source for the explicit Run (stable identity per keystroke —
   * the memoized StatusBar keeps its onRun across typing) */
  const sourceRef = useRef(source);
  sourceRef.current = source;

  // ---- the virtual dep world (phase 5): the in-memory `$RUT_HOME` +
  // workspace the walk reads; the manifest/lock/demo rows are demo
  // state the env serves at their walked paths ----
  const envRef = useRef<MemoryEnv | null>(null);
  const manifestRowsRef = useRef(initialDepRows());
  const lockRowsRef = useRef(initialLockRows());
  /** the walk may only run once the std boot settles — a premature
   * walk would refuse every row (the store is still empty) and the
   * panel would blink nine honest-looking lies */
  const walkReady = useRef(false);
  const [depRows, setDepRows] = useState<DepRowView[]>(() =>
    STD_PINS.map((p) => ({ name: p.name, url: p.url, sha256: p.sha256, status: "fetching", std: true })),
  );
  const [notices, setNotices] = useState<string[]>([]);
  const [addState, setAddState] = useState<string | null>(null);
  if (envRef.current === null) {
    const env = new MemoryEnv();
    env.onNotify = (message) => {
      setNotices((prev) =>
        prev.includes(message) ? prev : [...prev, message].slice(-8),
      );
    };
    env.setFile(MANIFEST_PATH, renderManifestText(manifestRowsRef.current));
    env.setFile(LOCK_PATH, renderLockText(lockRowsRef.current));
    env.setFile(PLAYGROUND_DOC, CASES[0].source);
    envRef.current = env;
  }
  const env = envRef.current;

  useEffect(() => {
    void Runner.boot().then(setRunner);
    // the binding boots ONCE per page (its arena/legend live for the
    // session); a failure is the visible error state, never a
    // monochrome shrug. The env rides `load`'s own lane — the walk's
    // read-only capability is in place before the first resolve.
    RutLsp.boot(undefined, env)
      .then((l) => {
        setLsp(l);
        setLspMode("live");
      })
      .catch((err: unknown) => {
        setLspDetail(err instanceof Error ? err.message : String(err));
        setLspMode("error");
      });
  }, [env]);

  // ---- the walk lane: the ACTOR owns the WHEN — `reloadDeps` defers the
  // walk under the core's own 200ms debounce window and coalesces (a
  // burst walks once over the freshest world; the clock is the env's
  // `wakeAfter`, an injected dumb timer). The promise IS the world
  // event (D2): the resolved `WalkOut` drives the deps panel's rows
  // and the world tick — `WalkOut.missing` is the panel's per-dep
  // truth. `null` means a later reload superseded this one (its
  // promise carries the world) — nothing to apply here ----
  const walkWorld = useCallback(async () => {
    if (!lsp) return;
    // the panel surfaces manifest errors: the App renders the
    // manifest text from its own rows, so a parse error is the
    // renderer's bug — surfaced honestly, and nothing walks
    const manifestText = renderManifestText(manifestRowsRef.current);
    const table = await lsp.parseManifest(manifestText);
    if (isEnvelope(table)) {
      setNotices((prev) =>
        prev.includes(table.error) ? prev : [...prev, table.error].slice(-8),
      );
      return; // a broken manifest walks nothing — the hint stands
    }
    const out = await lsp.reloadDeps([WORKSPACE_ROOT]);
    if (out === null || isEnvelope(out)) {
      return; // superseded (the LAST reload's promise carries the world)
    }
    setDepRows((prev) =>
      prev.map((r) => {
        // a FAILED fetch keeps its own reason — the walk's refusal
        // would only name the empty store beneath it
        if (r.status === "failed") return r;
        const missing = out.missing.find((m) => m.url === r.url);
        if (missing === undefined) {
          return { ...r, status: "mounted", note: undefined };
        }
        return {
          ...r,
          status: "refused",
          note:
            missing.sha256 === null
              ? "no sha256 pin in rut.lock"
              : "not in the store (or the pin does not verify)",
        };
      }),
    );
    // the world moved: re-analyze the standing source (the check
    // lane's answer changed), never re-run it
    immediateRunSrc.current = sourceRef.current;
    setWorldTick((t) => t + 1);
  }, [lsp]);

  // ---- the std boot: fetch the nine zips from the CDN in parallel,
  // verify each against the pin table, install into the virtual store
  // — then (and only then) the first walk runs ----
  useEffect(() => {
    if (!lsp) return;
    let cancelled = false;
    void (async () => {
      const settled = await Promise.all(
        STD_PINS.map(async (pin) => {
          try {
            const res = await fetch(pin.url);
            if (!res.ok) throw new Error(`HTTP ${res.status}`);
            const bytes = new Uint8Array(await res.arrayBuffer());
            const sha = await sha256Hex(bytes);
            if (sha !== pin.sha256) {
              throw new Error(`sha256 mismatch (fetched ${sha.slice(0, 8)}…)`);
            }
            const name = await env.install(pin.url, bytes, pin.name);
            return { pin, name, err: null as string | null };
          } catch (err) {
            return {
              pin,
              name: pin.name,
              err: err instanceof Error ? err.message : String(err),
            };
          }
        }),
      );
      if (cancelled) return;
      setDepRows((prev) =>
        prev.map((r) => {
          const hit = settled.find((s) => s.pin.url === r.url);
          if (hit === undefined) return r;
          return hit.err === null
            ? { ...r, name: hit.name, status: "installed" }
            : { ...r, status: "failed", note: hit.err };
        }),
      );
      walkReady.current = true;
      void walkWorld();
    })();
    return () => {
      cancelled = true;
    };
  }, [lsp, env, walkWorld]);

  // ---- the custom-dep lane: fetch a zip url, install it into the
  // store, declare it (manifest row + lock row at the computed sha),
  // and re-walk — exactly the `rut add`-shaped flow over the same
  // read-only walk ----
  const addCustomDep = useCallback(
    (url: string) => {
      if (!lsp) return;
      setAddState(`fetching ${url}…`);
      void (async () => {
        try {
          const res = await fetch(url);
          if (!res.ok) throw new Error(`HTTP ${res.status}`);
          const bytes = new Uint8Array(await res.arrayBuffer());
          const fallback =
            url.split("/").pop()?.replace(/\.zip$/, "") ?? "dep";
          setAddState(`installing ${fallback}…`);
          const name = await env.install(url, bytes, fallback);
          const sha = await sha256Hex(bytes);
          // declare: the manifest row (replacing any earlier row at
          // the same url) + the lock pin. A name another row already
          // mounted keeps its first mount (the walk's
          // first-mount-wins law — the world serves the standing pin)
          manifestRowsRef.current = [
            ...manifestRowsRef.current.filter((r) => r.url !== url),
            { name, url },
          ];
          lockRowsRef.current = [
            ...lockRowsRef.current.filter((r) => r.url !== url),
            { pkg: name, url, sha256: sha },
          ];
          env.setFile(MANIFEST_PATH, renderManifestText(manifestRowsRef.current));
          env.setFile(LOCK_PATH, renderLockText(lockRowsRef.current));
          setDepRows((prev) => [
            ...prev.filter((r) => r.url !== url),
            { name, url, sha256: sha, status: "installed", std: false },
          ]);
          setAddState(null);
          void walkWorld();
        } catch (err) {
          setAddState(null);
          setDepRows((prev) => [
            ...prev,
            {
              name: url.split("/").pop()?.replace(/\.zip$/, "") ?? url,
              url,
              sha256: null,
              status: "failed",
              note: err instanceof Error ? err.message : String(err),
              std: false,
            },
          ]);
        }
      })();
    },
    [lsp, env, walkWorld],
  );

  // the doc IS the workspace entry: every buffer move (an edit or a
  // case switch) lands in the env and reloads the deps, so the doc's
  // defs row and the check lane ride the live text
  useEffect(() => {
    env.setFile(PLAYGROUND_DOC, source);
    if (walkReady.current) void walkWorld();
  }, [source, env, walkWorld]);

  const applyCompileFailure = useCallback((diags: Diag[]) => {
    setPanes({ output: [], irDump: "", diags });
    setFuelUsed(0);
    setHeapUsed(0);
    setParked(false);
  }, []);

  const applyRun = useCallback(
    (res: RunResult, compiled: CompileResult | null, src: string) => {
      if (compiled) {
        lastCompiled.current = compiled;
        lastCompiledSrc.current = src;
      }
      const shown = compiled ?? lastCompiled.current;
      setPanes({
        output: res.output,
        ast: shown?.ast,
        astSrc: shown ? lastCompiledSrc.current : "",
        irDump: shown?.irDump ?? "",
        trap: res.trap,
        err: res.err,
      });
      setFuelUsed(res.fuelUsed);
      setHeapUsed(res.heapBytes);
      setParked(res.parked === true);
    },
    [],
  );

  /** the one compile+run path (survey D1): every run — explicit or
   * auto — compiles the given source and runs it at the given fuel
   * budget (the heap box is shared), then applies through applyRun.
   * LAST-WINS: each run bumps the token, a superseded run's applies
   * die on the token checks. */
  const executeRun = useCallback(
    (src: string, fuelBudget: number) => {
      if (!runner) return;
      const token = ++runToken.current;
      setRunning(true);
      try {
        // compile (a fatal compile is LOUD: the diags become the output)
        let compiled: CompileResult;
        try {
          compiled = runner.compile(src);
        } catch (err) {
          if (token !== runToken.current) return;
          setPanes({
            output: [`compile failed: ${String(err)}`],
            irDump: "",
          });
          setRunning(false);
          return;
        }
        if (token !== runToken.current) return;
        if (!compiled.binary) {
          applyCompileFailure(compiled.diags);
          return;
        }
        const res = runner.run(
          compiled.binary,
          { fuel: fuelBudget, heapBytes: budget.heapBytes },
        );
        if (token !== runToken.current) return; // stale
        applyRun(res, compiled, src);
      } finally {
        if (token === runToken.current) setRunning(false);
      }
    },
    [runner, budget, applyRun, applyCompileFailure],
  );

  /** ▶ Run: uncapped (fuel 0) unless the user enabled the fuel box —
   * auto-runs never touch this lane */
  const run = useCallback(() => {
    executeRun(sourceRef.current, fuelEnabled ? budget.fuel : 0);
  }, [executeRun, budget, fuelEnabled]);

  /** the D-1 auto-run: parked frames are DROPPED first (a Resume
   * pointing at a frame from a different source is a correctness trap —
   * the law selectCase already follows), then the reduced auto budget. */
  const runAuto = useCallback(
    (src: string) => {
      runner?.dropFrame();
      setParked(false);
      executeRun(src, AUTO_FUEL);
    },
    [runner, executeRun],
  );
  /** the debounced lane reads the LATEST runAuto at fire time, so a
   * budget edit never re-arms the lane for an unchanged source */
  const runAutoRef = useRef(runAuto);
  runAutoRef.current = runAuto;

  // the debounced FULL analyze per change (survey D3): case selects and
  // edits both flow through `source`, so this one effect covers both —
  // and the SAME timer is the D-1 auto-run lane (one timer, two
  // consumers; LAST-WINS: a newer edit clears this timer and re-arms —
  // runs are never queued, a superseded run dies on the token check).
  // The analyze is an async bridge now (the actor seam): one promise,
  // the stale flag still decides whether its answer may land.
  useEffect(() => {
    if (!lsp) return;
    let stale = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const [a, legend] = await Promise.all([
            lsp.analyze(PLAYGROUND_DOC, source),
            lsp.legend,
          ]);
          if (stale) return;
          overlayCache.current = buildOverlayCached(
            source,
            decodeTokens(a.tokens.data),
            legend,
            a.diags,
            overlayCache.current,
          );
          setHighlight({
            lines: overlayCache.current.lines,
            diags: a.diags,
          });
        } catch (err) {
          // a failed analyze must never kill the page — loud in the
          // console, monochrome in the editor (the honest interim)
          console.error("rut-lsp analyze failed:", err);
          if (!stale) setHighlight(null);
        }
        // the auto-run consumer — independent of the highlight outcome.
        // A case switch (or the boot install) skips here: its run was
        // IMMEDIATE for this exact source and the panes already hold it.
        if (stale) return;
        const skip = immediateRunSrc.current === source;
        immediateRunSrc.current = null;
        if (!skip) runAutoRef.current(source);
      })();
    }, ANALYZE_DEBOUNCE_MS);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [lsp, source, worldTick]);

  const selectCase = useCallback(
    (c: RutCase) => {
      // a case switch must not inherit the previous case's machine —
      // and the fresh case's run starts with no frame at all
      runner?.dropFrame();
      lastCompiled.current = null;
      lastCompiledSrc.current = "";
      setCurrentCase(c);
      setSource(c.source);
      setPanes(EMPTY_PANES);
      setFuelUsed(0);
      setHeapUsed(0);
      setParked(false);
      // the D-1 design, pinned: the case switch runs IMMEDIATELY (the
      // panes populate the moment a case opens; the fuel demo parks at
      // once and teaches the trap on selection) — and the debounced
      // lane skips this exact source so it never runs twice.
      immediateRunSrc.current = c.source;
      executeRun(c.source, AUTO_FUEL);
    },
    [runner, executeRun],
  );

  const editSource = useCallback((s: string) => {
    setSource(s);
    // an edit re-tags the lane as an EDIT lane: even if it lands before
    // the case switch's debounced fire, the auto-run must go out
    immediateRunSrc.current = null;
  }, []);

  const resume = useCallback(
    () => {
      if (!runner) return;
      const token = ++runToken.current;
      setRunning(true);
      try {
        // REAL resume (survey D5): the parked frame continues — never a
        // re-run. +10M ops, same as the button names.
        const extra = 10_000_000;
        const res = runner.resume(extra);
        if (token !== runToken.current) return;
        setBudget((b) => ({ ...b, fuel: b.fuel + extra }));
        applyRun(res, null, "");
      } finally {
        if (token === runToken.current) setRunning(false);
      }
    },
    [runner, budget, applyRun],
  );

  /** the editor popup's query — the mounted world answers through the
   * binding's `complete` (the doc-sync law: the live text rides) */
  const requestCompletions = useCallback(
    async (line: number, character: number, src: string) => {
      if (!lsp) return [];
      return lsp.complete(PLAYGROUND_DOC, line, character, src);
    },
    [lsp],
  );

  const banner = useMemo(() => {
    if (!runner) return "booting…";
    return runner.state.mode === "wasm" ? runner.state.banner : "";
  }, [runner]);

  /** the deps panel's status line — the boot lane's one honest sentence */
  const depsStatusLine = useMemo(() => {
    const fetching = depRows.filter((r) => r.status === "fetching").length;
    const failed = depRows.filter((r) => r.status === "failed").length;
    if (fetching > 0) {
      return `std deps: fetching from CDN (${depRows.length - fetching}/${depRows.length})…`;
    }
    if (failed === depRows.length && depRows.length > 0) {
      return "std deps: CDN fetch failed — the page runs std-less";
    }
    if (failed > 0) {
      return `std deps: ${failed} fetch failed — the rest ride the dep lane`;
    }
    return "std deps: riding the dep lane (manifest + store + walk)";
  }, [depRows]);

  if (runner && runner.state.mode === "error") {
    return <BootError banner={runner.state.banner} />;
  }
  if (lspMode === "error") {
    return (
      <LspError
        detail={`${lspDetail} — run \`${BUILD_WASM_COMMAND}\` in demo/`}
      />
    );
  }

  return (
    <div className="app">
      <header className="app-header">
        <h1>rut playground</h1>
        <span className="app-sub">
          React + rspack · rut as wasm · budgets enforced by the host
        </span>
      </header>

      {banner && <div className="banner">{banner}</div>}

      <main className="app-main">
        <div className="app-side">
          <CaseList
            groups={groups}
            selectedId={currentCase.id}
            onSelect={selectCase}
          />
          <DepsPanel
            rows={depRows}
            statusLine={depsStatusLine}
            notices={notices}
            onAdd={addCustomDep}
            addState={addState}
          />
        </div>
        <Editor
          value={source}
          onChange={editSource}
          highlight={highlight}
          completions={requestCompletions}
        />
        <Panes data={panes} />
      </main>

      <StatusBar
        running={running}
        budget={budget}
        onBudgetChange={setBudget}
        fuelEnabled={fuelEnabled}
        onFuelEnabledChange={setFuelEnabled}
        onRun={run}
        onResume={resume}
        canResume={parked && panes.trap === "OutOfFuel"}
        fuelUsed={fuelUsed}
        heapUsed={heapUsed}
        trap={panes.trap}
      />
    </div>
  );
}
