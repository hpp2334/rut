# dep-kinds — the batch report

The record of the dep-kinds batch: four phases, one shared tree, base
`2eaefd8`, all on `master`. Companion to
`docs/dep-kinds-survey.md` (phase 0 — the census, the mechanism design,
the pinned matrix and grammar; its §-numbers are the ones the landed
diagnostics cite). The law lives in `rfc/0045-dependency-kinds.md`;
this report is the record.

## 1. The story, in one section

Rut packages could relate to each other exactly one way: `[deps]`,
transitively mounted. The batch asked what it costs to add the two
missing relations — "required of my consumer" and "only while
developing me" — without breaking the loader, the splice graph, or a
single existing program, and landed the answers in four commits:

1. **What does the tree actually do today?** (phase 0, `9c7e435` —
   the survey.) The manifest parser is a hand-rolled TOML subset;
   `[peer-deps]` was an *unknown section* error; inline-table values
   were strings-only, so even `optional = true` could not parse; the
   splice graph carried pre-combined per-use text with NO cross-use
   dedup — store + t1 both ride pouch, so a consumer importing both
   would define `Vec` twice (the landed `appkit` workaround in
   `05-todolist-web` collapsed sibling uses to dodge it). The ruling,
   the pinned grammar, and the six-row missing-peer matrix went in
   verbatim; the design sections (descriptor `lib` groups, the
   four-pass mount order, dedup-by-origin, D1–D4, bundle v3) were
   flagged as proposals for the implement phases to ratify.
2. **The loader learns the kinds.** (phase 1, `de228e1`.) The three
   tables parse — `optional` is the one bool the inline-table grammar
   learns; `[deps]` rejects it; `lib` must be a `.rut` (a `.d.rut`
   decl surface does not gate); unknown descriptor keys are the
   `[entry]` strictness; the cross-table collision is D4, with peer+dev
   the sanctioned pairing. The four mount passes landed in
   `load_dir_session`'s land with zero graph change: the `[deps]` walk
   (now recording each mounted pkg's peer declarations), the dev pass
   (root-only), the peer gate (one post-closure pass — D1 loud for a
   missing required peer, presence-based group appends for present
   peers, inert when an optional peer is absent, D3 for the root's own
   broken peer paths), and unchanged compile. The Session gained the
   `PeerDecl` registry. Bundles stayed v2 and REFUSE a peer-deps
   manifest loudly (groups need v3). Fixtures T1–T9 + T11 green.
3. **The splice dedups; the reference site learns the peer; bundles
   carry groups.** (phase 2, `ab64535`.) `Unit::Inline` now carries
   its ordered `(origin spec, source)` leaf list and composition skips
   specs already present — first position wins, topological order
   preserved; the t1 collision shape became structurally impossible
   (T10, with the control proving it was real), and in the
   no-collision case the composed text is BYTE-IDENTICAL to the old
   law (T13). D2 landed: `resolve`'s miss path consults the registry,
   so referencing an absent optional peer names pkg + peer + the
   integration + the fix — never the bare NoModule. Bundle format
   went 2 → 3 (groups ride the archive; old loaders refuse by the
   gate they already had); the 03-plugin example's manifest bumped
   accordingly. VERSION stayed 8.
4. **The record.** (phase 3, this commit — docs only.) RFC 0045 (the
   law the diagnostics already cite), the amendments to RFC 0038
   (the v3 ledger row), RFC 0041 (the manifest section's touchpoints),
   and RFC 0012 (peer-gated impls stay trait-local; the gate precedes
   the placement check); the examples/README section; this report.

## 2. The matrix, as executed

The survey's test plan (§3) vs the landed suite. Every row cites the
matrix cell it pins — the suite IS the ruling, executable.

| T | the survey's plan | matrix cell | the test that pins it |
|---|---|---|---|
| T1 | optional peers absent, never touched | row 2 — silent success | `peer_deps::t1_optional_peers_absent_is_silent` |
| T2 | peer present → group mounts, dispatches | row 4 — presence-based | `peer_deps::t2_peer_present_group_mounts_and_dispatches` |
| T3 | both peers, name order | row 4 — presence-based (×2) | `peer_deps::t3_both_peers_mount_in_name_order` |
| T4 | reference with peer absent → D2 | row 3 — the dedicated diag | `peer_deps::t4_reference_with_peer_absent_gets_the_dedicated_diag` |
| T5 | required peer missing → D1 loud | row 1 — loud, not auto-pulled | `peer_deps::t5_required_peer_missing_is_loud_d1` |
| T6 | self-build: dev-deps guarantee presence | row 5 — no missing case | `peer_deps::t6_self_build_dev_deps_guarantee_presence` |
| T7 | broken peer path: D3 at self-build, inert for consumers | row 6 — packaging bug | `t7_broken_peer_path_is_loud_d3_at_self_build` + `t7b_broken_peer_path_is_inert_for_consumers` + `t7c_presence_is_by_name_the_path_is_never_read` |
| T8 | peer-of-peer chains through one pass | row 4 at depth (one pass = fixpoint) | `peer_deps::t8_peer_gate_is_one_post_closure_pass` |
| T9 | both-kinds pairing end-to-end | row 5 + §2's pairing law | `peer_deps::t9_both_kinds_pairing_end_to_end` |
| T10 | the t1 collision shape compiles green | not a matrix row — survey §2.4's dedup | `dep_dedup::t10_sibling_imports_of_a_shared_inline_pkg_compile` + the control `t10_the_collision_was_real` |
| T11 | D4 + the line-targeted grammar errors | rows 1/6's loudness, grammar law | `peer_deps::t11_d4_through_the_loader` + `session.rs`'s parser unit tests |
| T12 | bundle v3 round-trip; refuse-never-guess | RFC 0038 §4 consistency | `dep_bundles::t12_pack_load_run_t2_world` + `t12_pack_load_run_t3_world` + `t12_refuse_never_guess` |
| T13 | regression: everything green untouched, seam byte-identical | the dedup's semantics-preservation | `dep_dedup::t13_no_collision_chain_compiles_byte_identically` + `t13_no_collision_multi_dep_composition_byte_identical` + the batch gates |

Landed beyond the plan: `peer_deps::mount_dir_mounts_no_dev_deps_and_runs_no_gate`
— the embedder-offer law (pass 2 is root-only and the gate is a
program-closure property, so `mount_dir` does neither; its peer
declarations are still recorded). The fixtures live under
`crates/rut-driver/tests/data/peers/` — hermetic mini-pkgs
(self-owned pouch/nmapset-shaped generics) so the suite never couples
to the real pkgs' evolution; `json/`'s manifest is the pinned grammar
plus the `lib` keys, and `json_required`/`json_broken` are the T5/T7
variants.

## 3. The dedup, in summary

The decision: **dedup by origin at mount time in the graph** —
`Unit::Inline` carries the ordered `(origin spec, own source)` leaf
list; composition extends from each dep's leaves skipping specs
already present. Correctness argument: a second splice of the same
origin can only duplicate definitions (items are order-independent —
forward references are legal) and never contributes a name the first
splice didn't; monomorphization instantiates per call site within the
unit regardless of how many times the template text appears. So the
dedup is semantics-preserving and strictly removes an error class.

The proof has two halves. T10: two sibling inline pkgs both riding a
shared transitive inline pkg, imported separately — the exact shape
`appkit` was built to avoid — compiles green and RUNS
(`cons_sibs ← sib_a + sib_b`); the control (`t10_the_collision_was_real`)
pins that the same text spliced twice is a compile error, so the trap
was armed before the dedup. T13: in the no-collision case, worlds
compiled through the new graph are byte-identical (equal binaries) to
the hand-written old-law composition (`extra + "\n" + src`), which is
why every existing program — and every bench pin — could not move.
The `appkit` workaround retires as a NECESSITY: it stays legal (one
use, one leaf) and the example is untouched — another lane's property.

## 4. Honest limits

- **The LSP cannot see peer gating.** The survey recorded it, and it
  stands: the LSP never parses manifests — it embeds the toolchain
  pkgs' sources by hand (`rut-lsp/src/std_surface.rs`) and indexes
  names. Peer groups are invisible to it until it parses manifests:
  a json group's impls complete and hover unconditionally, and an
  absent-optional-peer reference squiggle-free in the editor can
  still fail loudly at compile. The compiler is the law; the LSP is
  advisory — but the gap is real and this batch did not close it
  (menu item below).
- **D1 is the loader's gate, not the Session's.** In a hand-mounted,
  gate-less world (an embedder registering modules directly), a
  missing REQUIRED peer surfaces as the bare unresolved-name miss —
  D2's dedicated text is reserved for optional peers, whose absence
  is legal. The gate runs where manifests are read
  (`load_dir_session`, the bundle load); `mount_dir` deliberately
  runs none (it is an offer to someone else's program, not a build).
- **Peer `path`s are directory-time metadata.** A dep's peer paths
  are never read — presence is by NAME (first-mount-wins guarantees
  the consumer's own path won). Only the program root's own peer
  paths are validated (D3), which is the pkg author's packaging-bug
  check, not a consumer guarantee.
- **Dev tables ride nothing.** A packed json carries its peer groups
  but never its dev-deps (pass 2 is root-only); consumers cannot see
  dev-only pkgs — by design, and untested-against-malice beyond the
  walk's structure (a dep declaring itself as someone's dev-dep
  simply never mounts).
- **The both-kinds pairing is one name, two tables** — sanctioned
  only for peer+dev. `[deps]` beside either stays the D4 error; there
  is no "transitive AND required" spelling, deliberately.

## 5. The delegated decision — where the README section lives

Recorded per the batch's conventions (a decision made by delegation,
with the trail):

- **The question** (put to the user mid-phase): scope item 2 said
  "THE README SECTION (where pkgs/manifests are documented)" — but the
  root `README.md` was deliberately emptied in `7fcae2f` ("root
  README cleared") and no README currently documents the manifest
  grammar. Options offered: `examples/README.md` (recommended — the
  examples are the runnable pkgs; `03-plugin`/`server` carry the
  in-tree `rut.toml`s; the appkit and v3-bump decision items are
  example-flavored), the root README (re-seeded), `demo/README.md`,
  or RFC 0041 §5 only.
- **The answer**: `examples/README.md` — the recommended option.
- **Who decided**: the orchestrator, on the user's behalf (overnight
  delegation), explicitly adopting the worker's recommendation:
  "the dep-kinds README section lands in examples/README.md … the
  root README was deliberately emptied (7fcae2f); examples/README.md
  is the live README and already carries the 03-plugin
  manifest/.rutbundle lane."
- **Where it landed**: the "Packages and manifests — the three dep
  kinds" section of `examples/README.md` — the three kinds, the json
  motivating case verbatim (the pinned grammar block), the decision
  record (dedup-by-origin / required-by-default / appkit's
  retirement-as-necessity / the plugin example's v3 bump), and the
  NOT-here note (registry/version-range deps).

## 6. The menu going forward

- **The LSP's peer-aware gating** — the honest gap above. Worth doing
  only when the LSP parses manifests (it would then gate completions
  / hovers by the session's peer registry); until then the gap is
  bounded (advisory surface only) and recorded in RFC 0045 §5/OQ-3.
- **rut/json, the real extraction** — the fixture world proves the
  law hermetically; the actual json pkg waits in the json lane with
  the survey's constraint recorded: the extracted base must be
  pouch-free (today's `Json` struct rides pouch's `Vec`), with every
  pouch-typed surface living in the peer group.
- **Registry/index and version-range deps** — deliberately NOT here
  (RFC 0045 OQ-1): `[peer-deps]` is a presence relation over mounted
  packages. The descriptor shape (key/value maps) leaves room for a
  future layer to land beside the tables without re-litigating the
  kinds.
- **`appkit`'s retirement-in-fact** — the workaround is now a choice,
  not a necessity (T10's shape composes). If the todolist-web lane
  ever refactors its mount, the single-splice wrapper can dissolve
  into `store` + `t1` uses; nothing in the engine needs it.
- **A manifest lint surface** (`rut check`) — the line-targeted
  errors are loud but load-time only; a pre-flight manifest check
  could name D3-class packaging bugs before a publish. Cheap, not
  urgent.

## 7. The gates, as landed per phase

| phase | commit | workspace tests | wasm32 | bench suite |
|---|---|---|---|---|
| 0 | `9c7e435` | docs-only, untouched (green at base) | n/a | n/a |
| 1 | `de228e1` | 696 passed / 0 failed | exit 0 | green, pins bit-identical (no runtime change by construction) |
| 2 | `ab64535` | 704 passed / 0 failed | exit 0 | green, all 28 pins bit-identical (the nmapset dir-shaped rows ride the new splice path) |
| 3 | this commit | docs-only; re-verified — 704 passed / 0 failed | re-verified, exit 0 | re-verified (paranoia — docs cannot move pins) |

Phase 3 changed no code. The RFC amendments record landed behavior;
the README section documents it; nothing here moves a pin. Scratch
under `/tmp/opencode/batch-dep-kinds/p3/`.
