/**
 * RutApi resolution — THE RUNNER LAW (survey D1): the page
 * runs REAL wasm or it shows the error. There is no preview fallback.
 * A missing/failing `rut.wasm` (404, network error, init throw,
 * a glue surface that doesn't match this contract) boots the page
 * into `mode: "error"` — a full-page panel naming the exact
 * `npm run build:wasm` command — and the panes never mount.
 *
 * The wasm module is the rut-wasm crate's `RutEngine` class over
 * wasm-bindgen: the glue (crates/rut-wasm/pkg/rut_wasm.js, gitignored
 * build output) is bundled here; `boot` fetches the artifact bytes and
 * hands them to the glue's `init`. `compile` answers the JSON envelope
 * as a string (binary rides base64); `run`/`resume` take the binary
 * bytes and the fuel/heap budgets (u64 = BigInt at the boundary).
 */

import init, { RutEngine } from "../../crates/rut-wasm/pkg/rut_wasm.js";
import type { CompileResult, RutApi, RunResult, Budget } from "./wasm/rut-api";

export interface RunnerState {
  mode: "wasm" | "error";
  banner: string;
}

/** the exact command the error panel and every loud failure name */
export const BUILD_WASM_COMMAND = "npm run build:wasm";

function decodeBase64(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

class WasmApi implements RutApi {
  private engine: RutEngine;

  constructor(engine: RutEngine) {
    this.engine = engine;
  }

  compile(src: string): CompileResult {
    const parsed = JSON.parse(this.engine.compile(src)) as CompileResult & {
      binary?: string;
    };
    return {
      diags: parsed.diags ?? [],
      ast: parsed.ast,
      irDump: parsed.irDump ?? "",
      binary: parsed.binary ? decodeBase64(parsed.binary) : undefined,
    };
  }

  run(binary: Uint8Array, budget: Budget): RunResult {
    const parsed = JSON.parse(
      this.engine.run(
        binary,
        BigInt(Math.max(0, Math.floor(budget.fuel))),
        BigInt(Math.max(0, Math.floor(budget.heapBytes))),
      ),
    ) as RunResult;
    return {
      output: parsed.output ?? [],
      trap: parsed.trap ?? undefined,
      err: parsed.err ?? undefined,
      fuelUsed: parsed.fuelUsed ?? 0,
      heapBytes: parsed.heapBytes ?? 0,
      parked: parsed.parked ?? false,
    };
  }

  resume(extraFuel: number): RunResult {
    const parsed = JSON.parse(
      this.engine.resume(BigInt(Math.max(0, Math.floor(extraFuel)))),
    ) as RunResult;
    return {
      output: parsed.output ?? [],
      trap: parsed.trap ?? undefined,
      err: parsed.err ?? undefined,
      fuelUsed: parsed.fuelUsed ?? 0,
      heapBytes: parsed.heapBytes ?? 0,
      parked: parsed.parked ?? false,
    };
  }

  dropFrame(): number {
    return this.engine.drop_frame();
  }
}

/** the browser byte source: same-origin fetch of the shipped artifact */
async function fetchBytes(url: string): Promise<ArrayBuffer> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`fetch ${url}: HTTP ${res.status}`);
  return res.arrayBuffer();
}

export class Runner {
  readonly state: RunnerState;
  private api: RutApi | null = null;

  private constructor(state: RunnerState, api: RutApi | null) {
    this.state = state;
    this.api = api;
  }

  /**
   * Boot the page. `load` overrides the byte source (the headless smoke
   * reads the same file from disk); everything after the bytes — init,
   * the engine construction — is the one shared path the browser uses.
   */
  static async boot(
    load?: (url: string) => Promise<ArrayBuffer>,
  ): Promise<Runner> {
    try {
      const bytes = await (load ?? fetchBytes)("rut.wasm");
      // the glue is bundled into this module; the bytes are the shipped
      // artifact — an invalid/missing file fails init LOUDLY here
      await init({ module_or_path: new Uint8Array(bytes) });
      const api = new WasmApi(new RutEngine());
      return new Runner(
        {
          mode: "wasm",
          banner:
            "live — rut.wasm · the playground slice: core, calc, rt, ink, pouch, nmapset mounted",
        },
        api,
      );
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      return new Runner(
        {
          mode: "error",
          banner:
            `rut.wasm missing or invalid — run \`${BUILD_WASM_COMMAND}\` in demo/ — ${detail}`,
        },
        null,
      );
    }
  }

  get isLive(): boolean {
    return this.api !== null;
  }

  private live(): RutApi {
    if (!this.api) {
      throw new Error(
        `runner is not live — the wasm module failed to boot; run \`${BUILD_WASM_COMMAND}\` in demo/`,
      );
    }
    return this.api;
  }

  compile(src: string): CompileResult {
    return this.live().compile(src);
  }

  run(binary: Uint8Array, budget: Budget): RunResult {
    return this.live().run(binary, budget);
  }

  /** continue the parked frame (survey D5) — accumulates output */
  resume(extraFuel: number): RunResult {
    return this.live().resume(extraFuel);
  }

  /** retire a parked frame (case switches must not inherit machines) */
  dropFrame(): void {
    this.api?.dropFrame();
  }
}
