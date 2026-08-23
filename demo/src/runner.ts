/**
 * RutApi resolution (RFC 0041 §3): probe public/rut.wasm at boot —
 * 200 → wasm mode; 404 → static-preview mode (expected output + banner).
 */

import type { CompileResult, RutApi, RunResult, Budget } from "./wasm/rut-api";
import { CASES, type RutCase } from "./cases";

export interface RunnerState {
  mode: "wasm" | "preview";
  banner: string;
}

export class Runner {
  readonly state: RunnerState;
  private api: RutApi | null = null;

  private constructor(state: RunnerState, api: RutApi | null) {
    this.state = state;
    this.api = api;
  }

  static async boot(): Promise<Runner> {
    try {
      const res = await fetch("rut.wasm");
      if (res.ok) {
        // The rut-wasm crate (RFC 0041 §2) will instantiate and expose
        // { compile, run } as plain exports over this bytes payload.
        const bytes = await res.arrayBuffer();
        const api = await instantiate(new Uint8Array(bytes));
        return new Runner({ mode: "wasm", banner: "" }, api);
      }
    } catch {
      /* fall through to preview mode */
    }
    return new Runner(
      {
        mode: "preview",
        banner:
          "static preview — wasm module not built (RFC 0041 §3). " +
          "Output below is the annotated expectation, not a live run.",
      },
      null,
    );
  }

  compile(src: string): CompileResult {
    if (this.api) return this.api.compile(src);
    return { diags: [], astDump: placeholder("AST"), irDump: placeholder("LIR") };
  }

  run(
    source: string,
    binary: Uint8Array | undefined,
    budget: Budget,
    currentCase: RutCase | undefined,
  ): RunResult {
    if (this.api && binary) {
      return this.api.run(binary, budget);
    }
    // preview mode: replay the case's annotated output
    const expected =
      currentCase?.expected ??
      CASES.find((c) => c.source === source)?.expected ??
      ["(no annotated expectation for custom sources in preview mode)"];
    const lines = [...expected];
    let trap: string | undefined;
    if (currentCase?.id === "fuel-demo") {
      lines.pop(); // the resume-hint line becomes the trap
      trap = "OutOfFuel";
    }
    return { output: lines, trap, fuelUsed: 0, heapBytes: 0 };
  }
}

function placeholder(what: string): string {
  return `(${what} dump requires the wasm build — RFC 0041 §3)`;
}

/**
 * Instantiate the wasm module. Until crates/rut-wasm exists this is
 * unreachable (boot() only calls it on a successful fetch), so the
 * signature doubles as the binding spec for the Rust side.
 */
async function instantiate(_bytes: Uint8Array): Promise<RutApi> {
  // const { instance } = await WebAssembly.instantiate(_bytes, {});
  // return instance.exports as unknown as RutApi;
  throw new Error("rut.wasm present but binding unimplemented — see RFC 0041 §2");
}
