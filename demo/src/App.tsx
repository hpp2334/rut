import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CASES, DEFAULT_BUDGET, type RutCase } from "./cases";
import { EXAMPLES } from "./examples";
import { BUILD_WASM_COMMAND, Runner } from "./runner";
import { verifyAgainstExpected, type VerifyResult } from "./verify";
import { CaseList, type CaseGroup } from "./components/CaseList";
import { Editor, type Highlight } from "./components/Editor";
import { Panes, type PaneData } from "./components/Panes";
import { StatusBar } from "./components/StatusBar";
import { PLAYGROUND_DOC, RutLsp, decodeTokens } from "./lsp/rut-lsp";
import { buildOverlayCached, type OverlayCache } from "./lsp/overlay";
import type { CompileResult, Diag, RunResult } from "./wasm/rut-api";

const EMPTY_PANES: PaneData = { output: [], irDump: "" };
/** the while-typing lane (survey D3, tuned by the audit's M-1): ONE
 * debounced timer, two consumers — the overlay rebuild and the D-1
 * auto-run. 120 ms: the analyze is 0.1 ms at case sizes, the budget is
 * all human patience; the number is the catch-up record, not a gate. */
const ANALYZE_DEBOUNCE_MS = 120;
/** the D-1 fuel stance: auto-runs (case switch + edit lane) execute at
 * a reduced budget so a stray long loop cannot eat the page; the fuel
 * box and the Run button keep the user's full budget untouched. The
 * chip names the fuel it verified at, so the reduction is never
 * silent — and fuel-demo's expected matches at ANY budget, so its
 * chip stays green on the auto lane. */
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
          <code>public/rut.wasm</code> — RFC 0041 §3)
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
          it to <code>public/rut-lsp.wasm</code> — RFC 0041 §3)
        </p>
      </div>
    </div>
  );
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
  const [budget, setBudget] = useState(DEFAULT_BUDGET);
  const [fuelUsed, setFuelUsed] = useState(0);
  const [heapUsed, setHeapUsed] = useState(0);
  const [parked, setParked] = useState(false);
  const [verify, setVerify] = useState<VerifyResult | null>(null);
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

  useEffect(() => {
    void Runner.boot().then(setRunner);
    // the binding boots ONCE per page (its arena/legend live for the
    // session); a failure is the visible error state, never a
    // monochrome shrug
    RutLsp.boot()
      .then((l) => {
        setLsp(l);
        setLspMode("live");
      })
      .catch((err: unknown) => {
        setLspDetail(err instanceof Error ? err.message : String(err));
        setLspMode("error");
      });
  }, []);

  const applyCompileFailure = useCallback((diags: Diag[]) => {
    setPanes({ output: [], irDump: "", diags });
    setFuelUsed(0);
    setHeapUsed(0);
    setParked(false);
    setVerify(null);
  }, []);

  const applyRun = useCallback(
    (res: RunResult, compiled: CompileResult | null, src: string, expected: string[], fuelAtVerify: number) => {
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
      // THE SIDECAR FLIP (survey D2): verify every real run against the
      // run's OWN case expected — passed IN by the launcher, never read
      // from state here, so a case switch's IMMEDIATE run (launched in
      // the same tick as setCurrentCase) cannot verify against the
      // PREVIOUS case's contract. Auto-runs ride this exact path, so
      // the chip updates per auto-run and names its fuel.
      setVerify(
        verifyAgainstExpected(res, expected, fuelAtVerify),
      );
    },
    [],
  );

  /** the one compile+run path (survey D1/D2): every run — explicit or
   * auto — compiles the given source, runs it at the given fuel budget
   * (the heap box is shared), and applies through applyRun. LAST-WINS:
   * each run bumps the token, a superseded run's applies die on the
   * token checks. */
  const executeRun = useCallback(
    (src: string, fuelBudget: number, expected: string[]) => {
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
        applyRun(res, compiled, src, expected, fuelBudget);
      } finally {
        if (token === runToken.current) setRunning(false);
      }
    },
    [runner, budget, applyRun, applyCompileFailure],
  );

  /** ▶ Run: the user's FULL box budget — auto-runs never touch it */
  const run = useCallback(() => {
    executeRun(sourceRef.current, budget.fuel, currentCase.expected);
  }, [executeRun, budget, currentCase]);

  /** the D-1 auto-run: parked frames are DROPPED first (a Resume
   * pointing at a frame from a different source is a correctness trap —
   * the law selectCase already follows), then the reduced auto budget. */
  const runAuto = useCallback(
    (src: string) => {
      runner?.dropFrame();
      setParked(false);
      executeRun(src, AUTO_FUEL, currentCase.expected);
    },
    [runner, executeRun, currentCase],
  );
  /** the debounced lane reads the LATEST runAuto at fire time, so a
   * budget edit never re-arms the lane for an unchanged source */
  const runAutoRef = useRef(runAuto);
  runAutoRef.current = runAuto;

  // the debounced FULL analyze per change (survey D3): case selects and
  // edits both flow through `source`, so this one effect covers both —
  // and the SAME timer is the D-1 auto-run lane (one timer, two
  // consumers; LAST-WINS: a newer edit clears this timer and re-arms —
  // runs are never queued, a superseded run dies on the token check)
  useEffect(() => {
    if (!lsp) return;
    let stale = false;
    const timer = window.setTimeout(() => {
      try {
        const a = lsp.analyze(PLAYGROUND_DOC, source);
        if (stale) return;
        overlayCache.current = buildOverlayCached(
          source,
          decodeTokens(a.tokens.data),
          lsp.legend,
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
    }, ANALYZE_DEBOUNCE_MS);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [lsp, source]);

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
      setVerify(null);
      // the D-1 design, pinned: the case switch runs IMMEDIATELY (the
      // chip + panes populate the moment a case opens; the fuel demo
      // parks at once and teaches the trap on selection) — and the
      // debounced lane skips this exact source so it never runs twice.
      // The run carries the FRESH case's expected — the state update
      // is still in flight, so the closure cannot be trusted for it.
      immediateRunSrc.current = c.source;
      executeRun(c.source, AUTO_FUEL, c.expected);
    },
    [runner, executeRun],
  );

  const editSource = useCallback((s: string) => {
    setSource(s);
    // an edit re-tags the lane as an EDIT lane: even if it lands before
    // the case switch's debounced fire, the auto-run must go out
    immediateRunSrc.current = null;
    // the verdict belongs to the last run; an edit retires it until the
    // next run compares fresh (an edited source that then mismatches
    // shows its diff — the honest signal, survey D2)
    setVerify(null);
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
        applyRun(res, null, "", currentCase.expected, budget.fuel + extra);
      } finally {
        if (token === runToken.current) setRunning(false);
      }
    },
    [runner, budget, applyRun, currentCase],
  );

  const banner = useMemo(() => {
    if (!runner) return "booting…";
    return runner.state.mode === "wasm" ? runner.state.banner : "";
  }, [runner]);

  if (runner && runner.state.mode === "error") {
    return <BootError banner={runner.state.banner} />;
  }
  if (lspMode === "error") {
    return (
      <LspError
        detail={
          `${lspDetail} — run \`${BUILD_WASM_COMMAND}\` in demo/ ` +
          `(RFC 0041 §3)`
        }
      />
    );
  }

  return (
    <div className="app">
      <header className="app-header">
        <h1>rut playground</h1>
        <span className="app-sub">
          React + rspack · rut as wasm · budgets per RFC 0040
        </span>
      </header>

      {banner && <div className="banner">{banner}</div>}

      <main className="app-main">
        <CaseList
          groups={groups}
          selectedId={currentCase.id}
          onSelect={selectCase}
        />
        <Editor value={source} onChange={editSource} highlight={highlight} />
        <Panes data={panes} verify={verify} />
      </main>

      <StatusBar
        running={running}
        budget={budget}
        onBudgetChange={setBudget}
        onRun={run}
        onResume={resume}
        canResume={parked && panes.trap === "OutOfFuel"}
        fuelUsed={fuelUsed}
        heapUsed={heapUsed}
        trap={panes.trap}
        verify={verify}
      />
    </div>
  );
}
