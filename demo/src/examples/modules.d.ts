/**
 * Ambient declaration for the raw-asset imports (rspack `asset/source`):
 * each classic example's source is imported as a plain string — the
 * playground edits the live files, not copies, and
 * `crates/rut-cli/tests/playground.rs` gates them against a real
 * pipeline run. (The `*.expected` ambient block died with the
 * sidecars — the no-sidecars batch; expected rides inline in the
 * case entries, so tsc rejects any leftover `.expected` import.)
 */

declare module "*.rut" {
  const source: string;
  export default source;
}
