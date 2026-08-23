/**
 * The rut wasm contract (RFC 0041 §3).
 *
 * `demo/public/rut.wasm` (built from the future `crates/rut-wasm`,
 * RFC 0041 §2) must export these. This file is the single source of
 * truth mirrored by RFC 0041 §3 — change both together.
 */

export interface Diag {
  /** byte/char span in the source, if known */
  start?: number;
  end?: number;
  msg: string;
}

export interface CompileResult {
  diags: Diag[];
  /** pretty-printed flat AST (RFC 0030 §5) */
  astDump: string;
  /** pretty-printed LIR / bytecode (RFC 0032) */
  irDump: string;
  /** serialized module binary (RFC 0033) — absent when diags are fatal */
  binary?: Uint8Array;
}

export interface Budget {
  /** ops the run may execute (RFC 0040 §2) */
  fuel: number;
  /** bytes the self-managed heap may carve (RFC 0039/0040 §1) */
  heapBytes: number;
}

export interface RunResult {
  /** lines captured from the guest's print() calls */
  output: string[];
  /** "OutOfFuel" | "OutOfMemory" | "Interrupted" | trap kind, if any */
  trap?: string;
  fuelUsed: number;
  heapBytes: number;
}

export interface RutApi {
  compile(src: string): CompileResult;
  run(binary: Uint8Array, budget: Budget): RunResult;
}
