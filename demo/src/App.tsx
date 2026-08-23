import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CASES, DEFAULT_BUDGET, type RutCase } from "./cases";
import { Runner } from "./runner";
import { CaseList } from "./components/CaseList";
import { Editor } from "./components/Editor";
import { Panes, type PaneData } from "./components/Panes";
import { StatusBar } from "./components/StatusBar";
import type { CompileResult } from "./wasm/rut-api";

const EMPTY_PANES: PaneData = { output: [], astDump: "", irDump: "" };

export function App(): JSX.Element {
  const [runner, setRunner] = useState<Runner | null>(null);
  const [currentCase, setCurrentCase] = useState<RutCase>(CASES[0]);
  const [source, setSource] = useState(CASES[0].source);
  const [panes, setPanes] = useState<PaneData>(EMPTY_PANES);
  const [budget, setBudget] = useState(DEFAULT_BUDGET);
  const [fuelUsed, setFuelUsed] = useState(0);
  const [heapUsed, setHeapUsed] = useState(0);
  const [running, setRunning] = useState(false);
  const runToken = useRef(0);

  useEffect(() => {
    void Runner.boot().then(setRunner);
  }, []);

  const selectCase = useCallback((c: RutCase) => {
    setCurrentCase(c);
    setSource(c.source);
    setPanes(EMPTY_PANES);
    setFuelUsed(0);
    setHeapUsed(0);
  }, []);

  const run = useCallback(
    (extraFuel = 0) => {
      if (!runner) return;
      const b =
        extraFuel > 0
          ? { ...budget, fuel: budget.fuel + extraFuel }
          : budget;
      if (extraFuel > 0) setBudget(b);
      const token = ++runToken.current;
      setRunning(true);

      // compile
      let compiled: CompileResult;
      try {
        compiled = runner.compile(source);
      } catch (err) {
        setPanes({
          output: [`compile failed: ${String(err)}`],
          astDump: "",
          irDump: "",
        });
        setRunning(false);
        return;
      }
      // run (sync in wasm mode; preview is sync by construction)
      const res = runner.run(source, compiled.binary, b, currentCase);
      if (token !== runToken.current) return; // stale
      setPanes({
        output: res.output,
        astDump: compiled.astDump,
        irDump: compiled.irDump,
        trap: res.trap,
      });
      setFuelUsed(res.fuelUsed);
      setHeapUsed(res.heapBytes);
      setRunning(false);
    },
    [runner, source, budget, currentCase],
  );

  const banner = useMemo(() => {
    if (!runner) return "booting…";
    return runner.state.banner;
  }, [runner]);

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
          cases={CASES}
          selectedId={currentCase.id}
          onSelect={selectCase}
        />
        <Editor value={source} onChange={setSource} />
        <Panes data={panes} />
      </main>

      <StatusBar
        running={running}
        budget={budget}
        onBudgetChange={setBudget}
        onRun={() => run()}
        onResume={() => run(10_000_000)}
        canResume={panes.trap === "OutOfFuel"}
        fuelUsed={fuelUsed}
        heapUsed={heapUsed}
        trap={panes.trap}
      />
    </div>
  );
}
