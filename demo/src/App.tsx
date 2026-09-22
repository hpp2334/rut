import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CASES, DEFAULT_BUDGET, type RutCase } from "./cases";
import { EXAMPLES } from "./examples";
import { BUILD_WASM_COMMAND, Runner } from "./runner";
import { verifyAgainstExpected, type VerifyResult } from "./verify";
import { CaseList, type CaseGroup } from "./components/CaseList";
import { Editor } from "./components/Editor";
import { Panes, type PaneData } from "./components/Panes";
import { StatusBar } from "./components/StatusBar";
import type { CompileResult, Diag, RunResult } from "./wasm/rut-api";

const EMPTY_PANES: PaneData = { output: [], irDump: "" };

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

  useEffect(() => {
    void Runner.boot().then(setRunner);
  }, []);

  const selectCase = useCallback(
    (c: RutCase) => {
      // a case switch must not inherit the previous case's machine
      runner?.dropFrame();
      lastCompiled.current = null;
      setCurrentCase(c);
      setSource(c.source);
      setPanes(EMPTY_PANES);
      setFuelUsed(0);
      setHeapUsed(0);
      setParked(false);
      setVerify(null);
    },
    [runner],
  );

  const editSource = useCallback((s: string) => {
    setSource(s);
    // the verdict belongs to the last run; an edit retires it until the
    // next run compares fresh (an edited source that then mismatches
    // shows its diff — the honest signal, survey D2)
    setVerify(null);
  }, []);

  const applyRun = useCallback(
    (res: RunResult, compiled: CompileResult | null, fuelAtVerify: number) => {
      if (compiled) lastCompiled.current = compiled;
      const shown = compiled ?? lastCompiled.current;
      setPanes({
        output: res.output,
        ast: shown?.ast,
        irDump: shown?.irDump ?? "",
        trap: res.trap,
      });
      setFuelUsed(res.fuelUsed);
      setHeapUsed(res.heapBytes);
      setParked(res.parked === true);
      // THE SIDECAR FLIP (survey D2): verify every real run against the
      // case's expected — accumulates across resumes (the wasm host
      // reports the accumulated lines)
      setVerify(
        verifyAgainstExpected(res, currentCase.expected, fuelAtVerify),
      );
    },
    [currentCase],
  );

  const applyCompileFailure = useCallback((diags: Diag[]) => {
    setPanes({ output: [], irDump: "", diags });
    setFuelUsed(0);
    setHeapUsed(0);
    setParked(false);
    setVerify(null);
  }, []);

  const run = useCallback(
    () => {
      if (!runner) return;
      const token = ++runToken.current;
      setRunning(true);
      try {
        // compile (a fatal compile is LOUD: the diags become the output)
        let compiled: CompileResult;
        try {
          compiled = runner.compile(source);
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
        const res = runner.run(compiled.binary, budget);
        if (token !== runToken.current) return; // stale
        applyRun(res, compiled, budget.fuel);
      } finally {
        if (token === runToken.current) setRunning(false);
      }
    },
    [runner, source, budget, applyRun, applyCompileFailure],
  );

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
        applyRun(res, null, budget.fuel + extra);
      } finally {
        if (token === runToken.current) setRunning(false);
      }
    },
    [runner, budget, applyRun],
  );

  const banner = useMemo(() => {
    if (!runner) return "booting…";
    return runner.state.mode === "wasm" ? runner.state.banner : "";
  }, [runner]);

  if (runner && runner.state.mode === "error") {
    return <BootError banner={runner.state.banner} />;
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
        <Editor value={source} onChange={editSource} />
        <Panes data={panes} source={source} verify={verify} />
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
