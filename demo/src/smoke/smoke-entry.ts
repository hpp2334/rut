/**
 * The smoke bundle's entry (survey D5/D2 + the runner law): exports the
 * EXACT surface the React app uses — the same Runner (boot, compile,
 * run, resume, dropFrame), the same CASES/EXAMPLES (raw sidecar
 * imports), the same verifier. rspack bundles it for node (CJS) so
 * scripts/smoke.mjs can drive the app's own load path headlessly — the
 * extension's out/wasm.js + e2e-wasm.js pattern.
 */
export { Runner, BUILD_WASM_COMMAND } from "../runner";
export { CASES, DEFAULT_BUDGET } from "../cases";
export { verifyAgainstExpected, renderedLines } from "../verify";
export { EXAMPLES } from "../examples";
