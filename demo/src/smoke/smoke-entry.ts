/**
 * The smoke bundle's entry (survey D5 + the runner law): exports the
 * EXACT surface the React app uses — the same Runner (boot, compile,
 * run, resume, dropFrame), the same RutLsp binding (boot, analyze,
 * complete, legend, decodeTokens), the same dep-lane pieces the deps
 * panel drives (the virtual store env + the std pin table), the same
 * CASES/EXAMPLES. rspack bundles it for node (CJS) so
 * scripts/smoke.mjs can drive the app's own load path headlessly —
 * the extension's out/wasm.js + e2e-wasm.js pattern.
 */
export { Runner, BUILD_WASM_COMMAND } from "../runner";
export { RutLsp, decodeTokens } from "../lsp/rut-lsp";
export { buildOverlay } from "../lsp/overlay";
export { CASES, DEFAULT_BUDGET } from "../cases";
export { EXAMPLES } from "../examples";
export {
  LOCK_PATH,
  MANIFEST_PATH,
  MemoryEnv,
  PLAYGROUND_DOC,
  renderLockText,
  renderManifestText,
  sha256Hex,
  storeDir,
} from "../lsp/store-env";
export { STD_PINS } from "../lsp/std-pins";
