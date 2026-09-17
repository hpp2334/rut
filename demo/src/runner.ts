/**
 * RutApi resolution (RFC 0041 §3): probe public/rut.wasm at boot —
 * 200 → wasm mode; 404 → static-preview mode (expected output + banner).
 *
 * The wasm module is the rut-wasm crate over a raw ABI (RFC 0041 §2):
 *   exports: memory, rut_alloc(len) -> ptr,
 *            rut_compile(src_ptr, src_len) -> envelope,
 *            rut_run(bin_ptr, bin_len, fuel, heap) -> envelope
 * An envelope is [u32 LE length][JSON bytes]; the compile envelope carries
 * the module binary base64-encoded (RFC 0033) — mirrored by rut-api.d.ts.
 */

import type { CompileResult, RutApi, RunResult, Budget } from "./wasm/rut-api";
import { CASES, type RutCase } from "./cases";

export interface RunnerState {
  mode: "wasm" | "preview";
  banner: string;
}

interface WasmExports {
  memory: WebAssembly.Memory;
  rut_alloc(len: number): number;
  rut_compile(srcPtr: number, srcLen: number): number;
  rut_run(
    binPtr: number,
    binLen: number,
    fuel: bigint,
    heap: bigint,
  ): number;
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
    };
  }
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
        const bytes = new Uint8Array(await res.arrayBuffer());
        const { instance } = await WebAssembly.instantiate(bytes, {});
        const api = new WasmApi(instance);
        return new Runner(
          {
            mode: "wasm",
            banner: "live — rut.wasm (M1 vertical slice: static core, no host modules)",
          },
          api,
        );
      }
    } catch {
      /* fall through to preview mode */
    }
    return new Runner(
      {
        mode: "preview",
        banner:
          "static preview — wasm module not built (run `npm run build:wasm` in demo/, RFC 0041 §3). " +
          "Output below is the annotated expectation, not a live run.",
      },
      null,
    );
  }

  compile(src: string): CompileResult {
    if (this.api) return this.api.compile(src);
    return { diags: [], irDump: placeholder("LIR") };
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
