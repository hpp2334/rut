import { memo } from "react";
import type { Budget } from "../wasm/rut-api";
import type { VerifyResult } from "../verify";

export const StatusBar = memo(function StatusBar(props: {
  running: boolean;
  budget: Budget;
  onBudgetChange: (b: Budget) => void;
  onRun: () => void;
  onResume: () => void;
  canResume: boolean;
  fuelUsed: number;
  heapUsed: number;
  trap?: string;
  verify?: VerifyResult | null;
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

      {/* the verify chip (survey D2): every real run is diffed against
          the case's inline expected; the chip names the fuel it
          verified at — including the auto budget (the D-1 stance) */}
      {props.verify && (
        <span
          className={"chip " + (props.verify.ok ? "chip-pass" : "chip-fail")}
          title={
            props.verify.ok
              ? `the real run matched the expected exactly (at ${props.verify.fuelAtVerify.toLocaleString()} fuel)`
              : `the real run differs from the expected — see the diff in the Output pane (at ${props.verify.fuelAtVerify.toLocaleString()} fuel)`
          }
        >
          {props.verify.ok
            ? `✓ matches expected @ ${(props.verify.fuelAtVerify / 1e6).toLocaleString()}M fuel`
            : `✗ ${props.verify.differ} line${props.verify.differ === 1 ? "" : "s"} differ @ ${(props.verify.fuelAtVerify / 1e6).toLocaleString()}M fuel`}
        </span>
      )}

      <span className="stat">fuel used: {props.fuelUsed.toLocaleString()}</span>
      <span className="stat">heap: {fmtBytes(props.heapUsed)}</span>
      <span className={"stat" + (props.trap ? " trap" : "")}>
        {props.trap ? `Trap::${props.trap}` : "—"}
      </span>
    </footer>
  );
});

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MiB`;
}
