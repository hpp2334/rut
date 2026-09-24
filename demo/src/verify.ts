/**
 * The verifier (survey D2): after every REAL run, the engine's actual
 * output is diffed against the case's INLINE expected (the no-sidecars
 * batch — the classics' blocks live in the case entries verbatim). The
 * expected is the contract; the chip tells the truth about it.
 *
 * The trap line participates: the Output pane renders
 * `Trap::<name>` as the last line, so an expected pins it by carrying the
 * exact `Trap::OutOfFuel` line (fuel-demo).
 *
 * Determinism note: every case is deterministic GIVEN its budget, and
 * the verify records the fuel it compared at. fuel-demo is the one
 * budget-dependent case — its expected pins the DEFAULT budget
 * (10_000_000); a raised fuel box (or a Resume, which accumulates) is a
 * different program-run and shows a diff BY DESIGN.
 */

import type { RunResult } from "./wasm/rut-api";

/** one line-paired row of the expected/got diff */
export interface DiffRow {
  /** 1-based line number */
  n: number;
  /** the run's line — undefined when the expected has an extra line */
  got?: string;
  /** the expected's line — undefined when the run produced an extra line */
  expected?: string;
  same: boolean;
}

export interface VerifyResult {
  ok: boolean;
  /** how many line pairs differ */
  differ: number;
  /** the fuel budget this verification compared at */
  fuelAtVerify: number;
  rows: DiffRow[];
}

/** The lines as the Output pane renders them: output + the trap line. */
export function renderedLines(res: RunResult): string[] {
  return res.trap ? [...res.output, `Trap::${res.trap}`] : [...res.output];
}

/** Diff a real run against a case's inline expected. */
export function verifyAgainstExpected(
  res: RunResult,
  expected: string[],
  fuelAtVerify: number,
): VerifyResult {
  const got = renderedLines(res);
  const len = Math.max(got.length, expected.length);
  const rows: DiffRow[] = [];
  let differ = 0;
  for (let i = 0; i < len; i++) {
    const g = got[i];
    const e = expected[i];
    const same = g !== undefined && e !== undefined && g === e;
    if (!same) differ += 1;
    rows.push({ n: i + 1, got: g, expected: e, same });
  }
  return { ok: differ === 0, differ, fuelAtVerify, rows };
}
