/**
 * Ambient declarations for the raw-asset imports (rspack `asset/source`):
 * each classic example's source and its expected-output sidecar are
 * imported as plain strings — the playground edits the live files, not
 * copies, and `crates/rut-cli/tests/playground.rs` gates both against a
 * real pipeline run.
 */

declare module "*.rut" {
  const source: string;
  export default source;
}

declare module "*.expected" {
  const expected: string;
  export default expected;
}
