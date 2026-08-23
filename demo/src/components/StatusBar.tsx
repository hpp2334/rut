import type { Budget } from "../wasm/rut-api";

export function StatusBar(props: {
  running: boolean;
  budget: Budget;
  onBudgetChange: (b: Budget) => void;
  onRun: () => void;
  onResume: () => void;
  canResume: boolean;
  fuelUsed: number;
  heapUsed: number;
  trap?: string;
}): JSX.Element {
  return (
    <footer className="status-bar">
      <button
        className="primary"
        disabled={props.running}
        onClick={props.onRun}
      >
        ▶ Run
      </button>
      <button disabled={!props.canResume || props.running} onClick={props.onResume}>
        ↻ Resume (+10M fuel)
      </button>

      <label className="budget">
        fuel
        <input
          type="number"
          min={1}
          step={1_000_000}
          value={props.budget.fuel}
          onChange={(e) =>
            props.onBudgetChange({
              ...props.budget,
              fuel: Math.max(1, Number(e.target.value) || 1),
            })
          }
        />
      </label>
      <label className="budget">
        heap bytes
        <input
          type="number"
          min={1}
          step={65536}
          value={props.budget.heapBytes}
          onChange={(e) =>
            props.onBudgetChange({
              ...props.budget,
              heapBytes: Math.max(1, Number(e.target.value) || 1),
            })
          }
        />
      </label>

      <span className="stat">fuel used: {props.fuelUsed.toLocaleString()}</span>
      <span className="stat">heap: {fmtBytes(props.heapUsed)}</span>
      <span className={"stat" + (props.trap ? " trap" : "")}>
        {props.trap ? `Trap::${props.trap}` : "—"}
      </span>
    </footer>
  );
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MiB`;
}
