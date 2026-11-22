---
name: Plan Rule
description: The rules for writing a plan in this repo — apply whenever a plan is being authored or revised (a "plan X" / "design X" / "how should we do X" ask, a design discussion converging on a decision, or any document meant to drive implementation later). A plan is a LAW plus its derived changes, never a patch list — find the mechanism that makes the problem a non-issue, cite ground truth verified this session (file:line), supersede the old law outright with NO backward compat (shims, dual paths, and kept-for-safety code are forbidden; pinned tests update in the same change), and package it as self-contained phases any implementer can execute without this conversation.
---

# Plan Rule

A plan is a **contract for an implementer who was not in this conversation** —
a fresh agent session, a reviewer, a colleague, or you next week. Whatever
executes a phase sees only the plan text, never the discussion that produced
it. Write for that reader. A saved plan is a proposal: nothing runs until
the user says so.

## The first rule: a plan states a law, not a list of edits

The systematic move is always the same shape:

1. **Diagnose the current law** that produced the observed behavior — the old
   rule, precedent, or table that MAKES the symptom the natural output of the
   system. A behavior with no owner is a bug in the diagnosis, not a freedom.
2. **Replace the law** with one mechanism, stated so a reviewer could check
   any edit against it. The new law is named and binding ("the one-name
   law", "the capsule law") — a principle, not a TODO.
3. **Derive the changes.** Every edit in the plan is a consequence of the new
   law. A reader should be able to ask "why does this file change?" and get
   the law as the answer, never "because the bug went away".

Reject the plan (yours or a draft you are reviewing) on these tells:

- **The patch tell** — a change justified only by its symptom: adds a flag,
  a special branch, an allowlist, a wrapper, a "for now". Either the new law
  demands that case (then it is derived, fine) or the change is a hack
  (then redesign until it disappears).
- **The fresh-restart test** — if this system were written from scratch
  today, with everything now known, would the planned shape be in it? If no,
  plan the removal-and-replacement, never the accommodation.
- **Additions without deletions** — a plan that only ADDS is suspect. The
  plan says what DIES: every old law, table, field, check, message, and test
  pin the new law replaces is listed for deletion in the same change.
  Supersede, never patch beside.
- **Two homes** — if two places must now be kept in sync by discipline, the
  plan is creating the next bug. Every rule gets one mechanism and one file,
  and the plan names the home.

## No backward compat

The repo's own law is "stale bytes re-pack, they never load back" — plans
inherit it. Not allowed in a plan: migration shims, adapters, dual code
paths, "keep the old message as a fallback", feature flags for gradual
rollover, deprecation windows, keeping dead code because a test mentions it.

- **Diagnostics texts, wire shapes, internal APIs, file layouts** — redesign
  freely. The plan states the new shape; nothing carries the old one along.
- **Pinned tests update in the same change.** The plan lists, by path:line,
  which pins flip, which die, and what each asserts NOW (keep the scenario,
  replace the expectation — see the one-name plan's test section for the
  shape).
- **Relaxations are legal outcomes.** If the new law makes an old refusal
  legal, say so plainly and pin the new legality with its own tests.
- **The only external contract is the user's own publishing acts.** When a
  change touches std surface, the plan includes the std re-cut note
  (`pack-std.cjs` → review → `publish-std.cjs`, the human acts) but designs
  no compat for old tags — old tags serve their own commits and nothing
  loads them back.

## Ground truth before design

No assumed facts. Every load-bearing claim about current code is verified in
THIS session and cited `file:line` — the plan carries a **Ground truth**
section with the evidence (or per-area citations for small plans).

- Call-site claims are grep-proven ("repo-wide grep confirmed these are the
  only two-arg call sites"), not remembered.
- Test pins are listed by path:line with what they currently assert.
- If a claim cannot be verified yet, the plan makes VERIFYING it an explicit
  phase — data first, no repo changes — never a silent assumption.
- Docs claims count too: cite the book page and line when the plan
  contradicts or relies on documented behavior.

## Format

Sections, in order — keep the phase structure greppable, so one phase's text
can be lifted out and worked from alone:

1. `# <title>` — the law in one line if possible.
2. `## Goal` — what and why: the itch, and the unsystematic thing observed
   (name the old law that owns it).
3. **The law / decision cascade** — bullets, ALL binding on the implementer.
   Keep the law separate from its derived consequences; consequences are
   checkable, the law is the judge.
4. `## Ground truth` — the verified evidence (see above).
5. **Changes / Acts / `## Phases`** — ordered phases; **each phase leaves
   every gate green**. Parallel work is tasks INSIDE a phase
   (`### Task A/B/C`), never phases in parallel:
   - tasks declare **disjoint file scopes**; overlapping scopes mean two
     phases, not two tasks;
   - tasks must not consume a sibling's MID-FLIGHT output; if B needs what A
     lands, that is sequential phases;
   - if a task only needs a sibling's FACTS (names, numbers, landed
     interfaces), the plan names the fact explicitly so it can be relayed
     without coupling the tasks;
   - 2–4 tasks is the sweet spot; more needs justification in the plan;
   - natural splits: measure vs implement vs document; disjoint modules; a
     gate-building phase (fixtures, falsifiers) before an engine phase.
6. `## Gates` — the exact commands, per AGENTS.md, and which lanes the
   change triggers: the baseline trio (`cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets`, `cargo nextest run`); the wasm
   release lane when lexer/parser/`rut-lsp-wasm`/TextMate move; `mdbook
   build` (en + zh) + pot re-extract + PO folds when prose moves;
   `book_blocks` when book code moves; `pack-std.cjs --check` when std
   surface moves.
7. `## Out of scope` — explicit non-goals, INCLUDING the human acts (std
   publish, version bumps, deploys) so no implementer drifts into them.

## Self-containment (the plan travels alone)

Zero conversation context rides with a phase. Therefore:

- Full paths, real identifiers, spelled-out signatures. No "as discussed",
  no "the approach we agreed on", no unresolved pronouns.
- Acceptance criteria are checkable statements — test names to add/flip,
  gates to pass, exact message shapes — never vibes ("make it clean").
- Each phase carries enough of the law to be implemented alone; repeat the
  binding constraints inside the phase rather than pointing back vaguely.
- Names and behaviors the plan coins (a new door, a message family) are
  defined once, precisely, where they are introduced.

## Size discipline

- **Small mechanical change** → a one-phase plan: Goal / Changes (numbered
  per file, verbatim signatures for new code) / Gates / Out of scope. A few
  thousand words at most; a plan is overhead past its risk.
- **Systemic change** → the full shape: law + ground truth + acts, each act
  leaving the gates green, dependency order stated ("Act 2 required by Act
  1, not polish").
- The plan is DONE when a reviewer could diff the landed code against the
  plan text and every line is either honored or consciously superseded by
  the plan's own words.

## Pre-flight audit — reject your own plan before saving

Run down the list; any NO means fix the plan first:

1. Does a stated law exist, and could a reviewer check every change against it?
2. Is any edit justified only by its symptom? (derive it or kill it)
3. Does the plan say what dies? (deletions listed beside additions)
4. Would the fresh-restart design contain this shape?
5. Is anything kept for compatibility with no living consumer? (delete it)
6. Are all load-bearing facts cited from this session's reading?
7. Do parallel tasks declare disjoint scopes and name their cross-task facts?
8. Does each phase leave every gate green, with gates spelled as commands?
9. Could a stranger implement it correctly from the text alone?
10. Is out-of-scope explicit, including the human acts?

## Saving

- Write to `~/.opencode/plan/<slug>.md` — the slug names the law
  (e.g. `one-name-law-and-ctor-hover.md`, `http-url-only-verbs.md`).
- Tell the user where the plan is. A saved plan is a proposal — how (and
  whether) it gets executed is the user's call, never implied by saving it.
