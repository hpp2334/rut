/**
 * RutApi resolution — THE RUNNER LAW (RFC 0041 §3, survey D1): the page
 * runs REAL wasm or it shows the error. There is no preview fallback.
 * A missing/failing `rut.wasm` (404, network error, instantiate throw,
 * an export surface that doesn't match this contract) boots the page
 * into `mode: "error"` — a full-page panel naming the exact
 * `npm run build:wasm` command — and the panes never mount.
 *
 * The wasm module is the rut-wasm crate over a raw ABI (RFC 0041 §2):
 *   exports: memory, rut_alloc(len) -> ptr,
 *            rut_compile(src_ptr, src_len) -> envelope,
 *            rut_run(bin_ptr, bin_len, fuel, heap) -> envelope,
 *            rut_resume(extra_fuel, heap) -> envelope,   (survey D5)
 *            rut_drop_frame() -> u32
 * An envelope is [u32 LE length][JSON bytes]; the compile envelope carries
 * the module binary base64-encoded (RFC 0033) — mirrored by rut-api.d.ts.
 */

import type { CompileResult, RutApi, RunResult, Budget } from "./wasm/rut-api";

export interface RunnerState {
  mode: "wasm" | "error";
  banner: string;
}

/** the exact command the error panel and every loud failure name */
export const BUILD_WASM_COMMAND = "npm run build:wasm";

interface WasmExports {
  memory: WebAssembly.Memory;
  rut_alloc(len: number): number;
  rut_compile(srcPtr: number, srcLen: number): number;
  rut_run(binPtr: number, binLen: number, fuel: bigint, heap: bigint): number;
  rut_resume(extraFuel: bigint, heap: bigint): number;
  rut_drop_frame(): number;
}

function decodeBase64(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

class WasmApi implements RutApi {
  private e: WasmExports;
  private enc = new TextEncoder();
  private dec = new TextDecoder();

  constructor(inst: WebAssembly.Instance) {
    this.e = inst.exports as unknown as WasmExports;
    // the artifact and the page ship together — an export surface that
    // doesn't match THIS contract is an invalid artifact (loud, boot
    // turns it into the error panel, never a half-working page)
    const names: (keyof WasmExports)[] = [
      "memory",
      "rut_alloc",
      "rut_compile",
      "rut_run",
      "rut_resume",
      "rut_drop_frame",
    ];
    for (const n of names) {
      if (n !== "memory" && typeof this.e[n] !== "function") {
        throw new Error(`invalid rut.wasm: missing export ${n}`);
      }
    }
  }

  private write(bytes: Uint8Array): number {
    const ptr = this.e.rut_alloc(bytes.length);
    if (ptr === 0) throw new Error("wasm heap exhausted");
    new Uint8Array(this.e.memory.buffer, ptr, bytes.length).set(bytes);
    return ptr;
  }

  private readEnvelope(ptr: number): string {
    const view = new DataView(this.e.memory.buffer, ptr, 4);
    const len = view.getUint32(0, true);
    return this.dec.decode(new Uint8Array(this.e.memory.buffer, ptr + 4, len));
  }

  compile(src: string): CompileResult {
    const bytes = this.enc.encode(src);
    const ptr = this.write(bytes);
    const resPtr = this.e.rut_compile(ptr, bytes.length);
    const json = this.readEnvelope(resPtr);
    const parsed = JSON.parse(json) as CompileResult & { binary?: string };
    return {
      diags: parsed.diags ?? [],
      ast: parsed.ast,
      irDump: parsed.irDump ?? "",
      binary: parsed.binary ? decodeBase64(parsed.binary) : undefined,
    };
  }

  run(binary: Uint8Array, budget: Budget): RunResult {
    const ptr = this.write(binary);
    const resPtr = this.e.rut_run(
      ptr,
      binary.length,
      BigInt(Math.max(0, Math.floor(budget.fuel))),
      BigInt(Math.max(0, Math.floor(budget.heapBytes))),
    );
    const parsed = JSON.parse(this.readEnvelope(resPtr)) as RunResult;
    return {
      output: parsed.output ?? [],
      trap: parsed.trap ?? undefined,
      fuelUsed: parsed.fuelUsed ?? 0,
      heapBytes: parsed.heapBytes ?? 0,
      parked: parsed.parked ?? false,
    };
  }

  resume(extraFuel: number): RunResult {
    const resPtr = this.e.rut_resume(
      BigInt(Math.max(0, Math.floor(extraFuel))),
      // the parked machine keeps the heap limit it was built with
      BigInt(0),
    );
    const parsed = JSON.parse(this.readEnvelope(resPtr)) as RunResult;
    return {
      output: parsed.output ?? [],
      trap: parsed.trap ?? undefined,
      fuelUsed: parsed.fuelUsed ?? 0,
      heapBytes: parsed.heapBytes ?? 0,
      parked: parsed.parked ?? false,
    };
  }

  dropFrame(): number {
    return this.e.rut_drop_frame();
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
   * reads the same file from disk); everything after the bytes —
   * instantiate, export validation, mode decision — is the one shared
   * path the browser uses.
   */
  static async boot(
    load?: (url: string) => Promise<ArrayBuffer>,
  ): Promise<Runner> {
    try {
      const bytes = await (load ?? fetchBytes)("rut.wasm");
      const { instance } = await WebAssembly.instantiate(bytes, {});
      const api = new WasmApi(instance);
      return new Runner(
        {
          mode: "wasm",
          banner:
            "live — rut.wasm · the playground slice: core, calc, rt, ink, pouch, nmapset mounted (RFC 0041 §3)",
        },
        api,
      );
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      return new Runner(
        {
          mode: "error",
          banner:
            `rut.wasm missing or invalid — run \`${BUILD_WASM_COMMAND}\` in demo/ ` +
            `(RFC 0041 §3): ${detail}`,
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
