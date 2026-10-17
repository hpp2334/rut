# Remaining work — plan 10: per-file use scoping + async exports

Status of this checkpoint: **phase 1 code is complete and landed** (see
`plan10-phase(1)` commit); one verification gate is still in flight; phases 2–4
remain. Plan: `/Users/a/.opencode/plan/per-file-use-scoping-and-async-exports.md`.

## Landed — phase 1: per-file dep bindings, duplicate-name errors, no ambient

- The extern value-lane tables are keyed `(file mod path, name)` — a `use`
  binds ITS OWN FILE; cross-file steals are gone. Ambient fallback is
  `AMBIENT_SITE` (core prelude rows carrying the ambient bit, and primitive
  impls) — nothing else.
- `use_bindings` (file / head / leaf / qualified / composed) built before the
  binding passes in `crates/rut-driver/src/compile/program.rs`; the driver's
  binding loops insert per `(file, leaf)` off it (flat rows and qualified host
  rows both converge on per-file keys now).
- Duplicate-name law (NAME-level, not pkg-level): a leaf spelled twice in one
  file errors at the second spelling — `use a::{x}; use a::{x, y}` and
  `use a::{x}; use b::{x}` are both refused; `use a::{x}; use a::{y}` is fine.
- `use_bound_here` is gone (the `None => true` ambient arm with it); terminal
  call-miss diagnostics gained the two hints (bound-in-another-module /
  no-use-names-it).
- Tests: 7 new in `crates/rut-driver/tests/mods.rs` (both-files-same-name,
  duplicate-leaf pair, unimported-with-hint, other-module hint, prelude
  ambient control, own+dep coexistence). Fixtures widened: `tests/select.rs`,
  `tests/own_package_uses.rs`, `crates/rut-cli/tests/boundary.rs`,
  `crates/rut-cli/tests/e2e.rs`.
- Suites green when last run: rut-driver 701, rut-cli 211 (incl.
  `book_blocks`), `mods` 51/51.

## In flight (check BEFORE starting phase 2)

- `cargo nextest run -p rut-lir -p rut-driver -p rut-cli -p rut-lsp` — the
  full 4-crate verification gate — was started by the phase-1 session and is
  still running (slow `--list` discovery over ~60 signed binaries on the
  external volume; the build itself finished). **If it reports anything red,
  fix before proceeding.** The phase-1 session (`ses_eb7f11284ffe9OpuE2sqfiyqHj`,
  idle) wrongly expected the background run to wake it — nothing wakes on
  background completion; read the output file directly.

## Remaining

1. **Phase 1 close-out**: the plan-alignment review loop (read-only reviewer
   vs the phase text; fix rounds until ALIGNED, HIGH: none) once the gate is
   green.
2. **Phase 2 — validated-must-bind**: every `use` leaf that binds nothing in
   its file errors AT THE USE LINE — private rows ("not exported; mark it
   `pub`") and the async gap ("`pub async fn` … do not cross package surfaces
   yet", message isolated in a helper for the phase-3 flip). Tests:
   `async_use_errors_at_use_line`, `private_use_errors_at_use_line`,
   `type_leaf_still_binds`, `anchor_no_longer_feeds_the_child`.
   Payload ready: `…/T/opencode/rut-batch-plan-impl/p10-phase2.txt`.
3. **Phase 3 — `pub async fn` crosses package surfaces**: surface rows carry
   the real `is_async` (the hardcoded `false` goes); consumer-side weaving via
   the owner-request/materialization rail (spike first: weave-owner-side vs
   desugar; record the choice). Pinned acceptance: the anchor-free world
   compiles AND provably calls the KIT wrapper; local async suites verbatim;
   wire discipline — repack + new `std-vNN` tag ONLY if std bytes change.
4. **Phase 4 — corpus sweep + docs + po + final battery**: workspace nextest;
   book pages for the new law (modules-and-visibility + imports/nameability),
   byte-stable pot regen + zh_CN.po fold, both editions build; demo smoke
   (baseline 294/295); `examples/05-todolist-web` e2e-browser; `pack-std.cjs
   --check` if phase 3 touched std.
5. Final batch report.

## Conventions in force

- Tests run via `cargo nextest run` — never bare `cargo test` (AGENTS.md).
- Port-22 SSH is blocked: push via
  `git push ssh://git@ssh.github.com:443/hpp2334/rut.git master`, verify with
  ls-remote against the same URL.
- Stage by explicit paths only; never `git add .`.
- Long gates: FOREGROUND tool calls with generous timeouts — backgrounded
  commands never wake the session.
