---
name: Batch Plan Implementation
description: Execute a multi-phase/step plan unattended — the orchestrator first switches its own session's model (keeping build mode) to the smarter plan model from opencode.jsonc, then creates a headless OpenCode session per phase and dispatches its prompt, while a small watch-only subagent polls every 60s, verifies the commit+push, and reports; continue to the next phase until the plan is done. No user interaction: decide autonomously, retry once, never delete sessions.
---

# Batch Plan Implementation

Drive an existing multi-phase/step plan to completion — **unattended**. The
user is usually away, so there is NO confirmation step and NO asking questions:
follow the autonomous decision policy below, record every decision in a run
log, and leave a complete report.

For each phase **you** (the session running this skill) create one fresh
headless OpenCode session and dispatch its prompt. A **watch-only subagent**
then monitors it (60s polling) and verifies the result. Phases run strictly
sequentially.

## Division of labor

| Actor | Does | Never does |
| --- | --- | --- |
| You (orchestrator) | plan parsing, session create, prompt dispatch, subagent spawn, batch control | implement a phase itself, delete a session |
| Subagent (one per phase, small prompt) | poll 60s, detect stall/timeout, verify commits + push + clean tree, extract summary, report | create sessions, send prompts, edit files, commit, delete sessions |

## Rules

- **Never ask the user anything.** Decide autonomously per the policy below
  and record the decision in the run log.
- **Handle anything that comes up yourself** (stalls, missing pushes, flaky
  failures) per the decision policy — never wait for the user.
- **Never delete a session.** "Closing" a phase just means moving on to the
  next one — every session stays in the session list for later inspection.
- One phase = one dedicated session = one watch subagent. Never batch phases.
- The subagent prompt is SMALL: session id + a few context values. The long
  phase text goes only into the dispatched session prompt, never into the
  subagent.
- Never implement a phase in this session. You orchestrate and verify only.
- Poll status every **60 seconds**. Do not use blocking waits.
- Pin the **default model from `opencode.jsonc`** on every dispatch — phase
  sessions and the watch subagent all use it (see step 3).
- **Switch your session's MODEL to the plan model first** (`agents.plan.model`
  in `opencode.jsonc` — the smarter model), **keeping the `build` agent/mode**,
  so all orchestration reasoning runs on it (see step 0).

## Autonomous decision policy

| Situation | Action |
| --- | --- |
| No plan found anywhere | Abort before touching anything; report where you looked. |
| Ambiguous phase boundaries/ordering | Best-effort split, record assumptions in the run log, proceed. |
| Dirty working tree | `git stash push --include-untracked -m "batch-plan-impl: auto-stash <date>"`, record in run log, proceed. |
| Phase FAILED (no commit / not pushed / dirty tree / outcome != succeeded) | Retry the phase ONCE with a fresh session and the same prompt. |
| Phase STALLED/TIMEOUT | Interrupt the session (`POST /api/session/$SID/interrupt`), then retry ONCE. |
| Phase fails after retry | **Stop the batch** — later phases likely depend on it. Leave everything for review. |
| Anything else unexpected | Choose the least destructive option, record it, keep going if safe. |

## Workflow

### 0. Switch this session's MODEL to the plan model (keep build mode)

Before any batch work, switch **your own session's model** to the smarter plan
model (`agents.plan.model` in `opencode.jsonc`, e.g. `zai-coding-plan/glm-5.3`)
so all orchestration reasoning runs on it.

**Model only — the session's agent/mode stays `build`.** Never switch the
agent to `plan`: that is a different thing (the read-only plan mode) and would
make this session unable to run tools. `POST /api/session/{id}/model` changes
only the model, so the agent is untouched. The switch applies to subsequent
turns.

```sh
cd "$(git rev-parse --show-toplevel)"   # config + sessions are location-scoped

# Your own session id: the most recently updated session RIGHT NOW —
# this very turn is updating it:
opencode api get /api/session \
  | jq -r '.data | sort_by(.time.updated) | reverse | .[0] | .id'   # -> $SELF

# The plan model from opencode.jsonc (strip // comments, then parse):
sed 's://.*$::' opencode.jsonc | jq -r '.agents.plan.model'   # -> $PLAN_REF

opencode api post /api/session/$SELF/model \
  --data "$(jq -n --arg ref "$PLAN_REF" \
    '{model:{providerID:($ref|split("/")[0]), id:($ref|split("/")[1])}}')"
```

Sanity-check `$SELF` against the session list if several sessions were touched
in the same second. If `agents.plan.model` is unset or the switch fails,
record it in the run log and continue on the current model — do not block the
batch on this.

### 1. Collect the plan

Identify the plan to execute, in this order:

1. The plan most recently produced in **this conversation** (e.g. by a plan
   agent or a previous discussion).
2. The newest obvious plan document (`rfc/`, `docs/`, `plans/`, `*.plan.md`,
   TODO files) — pick the most recently modified if several.

Extract an ordered list of phases/steps. For each, capture:

- `n` — ordinal
- `title` — short name
- `detail` — the FULL text of that phase/step from the plan, verbatim
  (acceptance criteria, file paths, spec excerpts — everything)

If none is found, abort and report. If boundaries/order are ambiguous, split
best-effort and record your assumptions.

### 2. Run log

Create `/tmp/opencode/batch-plan-impl/run.md` and record, as you go: the plan
source, the parsed phase list, every autonomous decision (with reason), each
session id, and each phase result. The final report is generated from this.

### 3. Preconditions

```sh
git status --porcelain          # if dirty: auto-stash (see policy), record it
git rev-parse HEAD              # record as $BASE
pwd                             # record as $PROJECT_DIR (absolute)
mkdir -p /tmp/opencode/batch-plan-impl   # scratch for payload files + run log
```

Read the **default model** (`model` in `opencode.jsonc`). The endpoint is
location-scoped, so run it with workdir = `$PROJECT_DIR`:

```sh
opencode api get /api/model/default
#   .data.providerID -> $MODEL_PROVIDER
#   .data.modelID    -> $MODEL_ID
#   $MODEL_JSON = {"providerID":"$MODEL_PROVIDER","id":"$MODEL_ID"}
#   $MODEL_REF  = "$MODEL_PROVIDER/$MODEL_ID"
```

### 4. Per phase (loop n = 1..N)

#### a. Create the session (you)

```sh
opencode api post /api/session \
  --data "$(jq -n --arg t "Phase <n>: <title>" --arg d "$PROJECT_DIR" \
            --arg m "$MODEL_JSON" \
            '{title:$t, location:{directory:$d}, model:($m|fromjson)}')"
```

Extract `$SID` via `jq '.data.id'`. `location.directory` is REQUIRED — without
it the session lands in the wrong directory. `model` pins the project default
model explicitly; do not rely on inheritance.

#### b. Dispatch the phase prompt (you)

Write the phase prompt to a payload file (never interpolate raw text into
`--data`), then dispatch and record the admission timestamp:

```sh
# phase<n>.txt content:
#   You are implementing phase <n> of an approved plan in this repository.
#
#   <detail>
#
#   Rules:
#   - Implement ONLY this phase. Do not start or anticipate other phases.
#   - Work within this repo. Keep changes minimal and consistent with existing code.
#   - When done, commit ALL changes with subject: "phase(<n>): <title>"
#     (plus a short body listing what was done).
#   - Then PUSH to the remote: git push (add -u <remote> <branch> if the
#     branch has no upstream yet). A phase is only done once pushed.
#   - Finish with a summary: what changed, files touched, test/build results.

jq -n --rawfile text /tmp/opencode/batch-plan-impl/phase<n>.txt '{text:$text}' \
  > /tmp/opencode/batch-plan-impl/phase<n>.json

opencode api post /api/session/$SID/prompt \
  --data "$(cat /tmp/opencode/batch-plan-impl/phase<n>.json)"
# record .data.time.created as $SINCE (epoch ms)
```

#### c. Spawn the watch subagent (you)

One subagent (subagent tool, `general` agent, foreground — phases are
sequential), spawned with the same pinned model (`model: "$MODEL_REF"` in the
subagent tool call). Fill only these placeholders: `$PROJECT_DIR`, `$SID`,
`$SINCE` (number), `$BASE`, `<n>`. Send exactly this small prompt:

```text
Watch worker for a headless OpenCode session. Observe and report ONLY — never
create sessions, send prompts, edit files, or change git state.

Context:
- Project dir: $PROJECT_DIR
- Session id: $SID   (prompt already dispatched — do not touch it)
- Prompt admitted at epoch ms: $SINCE
- Base commit: $BASE ; expected commit subject prefix: "phase(<n>):"

WATCH — repeat until finished (each step is its own shell call):
  sleep 60
  opencode api session.message.list \
    --param sessionID="$SID" --param order=desc --param limit=1 \
    | jq -e '.data[0].type=="idle" and .data[0].time.created > $SINCE' >/dev/null
  exit 0 => loop finished (DONE)
- Newest message `idle` after $SINCE means the agent loop finished.
- Do NOT use /api/session/active — it does not track headless sessions.
- STALLED if the newest message id stops changing for 20 consecutive polls
  while not idle.   TIMEOUT after 240 polls (~4h) with no idle.

ON DONE, collect:
1. Outcome:
   opencode api session.message.list \
     --param sessionID="$SID" --param order=desc --param limit=5 \
     | jq -r '.data[] | select(.type=="idle") | .outcome'     # want: succeeded
2. Verification:
   git -C $PROJECT_DIR log --oneline $BASE..HEAD
   git -C $PROJECT_DIR status -sb        # first line must NOT contain [ahead]
   git -C $PROJECT_DIR status --porcelain
3. Final assistant summary:
   opencode api session.message.list \
     --param sessionID="$SID" --param order=desc --param limit=15 \
     | jq -r '[.data[] | select(.type=="assistant")][0]
              | [.content[] | select(.type=="text") | .text] | join("\n")'

RESPOND with exactly this and nothing else:
STATUS: DONE | STALLED | TIMEOUT
OUTCOME: <idle outcome or "n/a">
COMMITS: <hashes + subjects, or "none">
PUSHED: <yes/no — no "[ahead" in status -sb>
DIRTY: <yes/no>
SUMMARY: <final assistant text, max ~30 lines>
```

#### d. Review and continue (you)

Phase succeeded only when ALL hold: `STATUS: DONE`, `OUTCOME: succeeded`,
at least one commit, `PUSHED: yes`, clean tree. Then leave the session as-is
and start the next phase (back to 4a).

Otherwise (FAILED / STALLED / TIMEOUT): retry ONCE per policy (fresh session,
same prompt; interrupt first if stalled). Retry succeeds → continue. Retry
fails too → **stop the batch** and go to the final report.

### 5. Final report

After the last phase (or on abort), summarize from the run log:

```
Batch plan complete: <k>/<N> phases done (base <BASE> -> HEAD <short hash>)
  1. <title>  ✓ <commit> pushed   ses_xxx
  2. <title>  ✗ STALLED (retried once, failed)   ses_yyy — left untouched
Overall: git diff --stat <BASE>..HEAD
Autonomous decisions: <auto-stash hash, retry counts, assumptions made>
All sessions were kept (never deleted) — resume any of them from the session list.
```

## Quick command reference

| Action | Actor | Command |
| --- | --- | --- |
| Switch own model to plan model | you | `opencode api post /api/session/$SELF/model --data '{"model":{"providerID":"<p>","id":"<m>"}}'` (agent stays `build`) |
| Read default model | you | `opencode api get /api/model/default` (workdir = `$PROJECT_DIR`) |
| Create session (pinned model) | you | `opencode api post /api/session --data "$(jq -n --arg t "..." --arg d "$PROJECT_DIR" --arg m "$MODEL_JSON" '{title:$t,location:{directory:$d},model:($m\|fromjson)}')"` |
| Dispatch prompt | you | `opencode api post /api/session/$SID/prompt --data "$(cat payload.json)"` |
| Poll newest message | subagent | `opencode api session.message.list --param sessionID=$SID --param order=desc --param limit=1` |
| Done when | subagent | newest msg `type=="idle"` and `time.created > $SINCE` |
| Verify | subagent | `git log --oneline $BASE..HEAD` + `status -sb` (no `[ahead`) + `status --porcelain` + idle `outcome` |
| Final summary text | subagent | list limit=15 → first `assistant` msg → join `content[]` where `type=="text"` |
| Interrupt stalled session | you | `opencode api post /api/session/$SID/interrupt` |

## Notes

- This project's `opencode.jsonc` allows all permissions
  (`"action": "*"` / `"effect": "allow"`), so phase sessions never pause on
  permission asks — no permission handling is needed anywhere in the batch.
- The default model comes from `opencode.jsonc` (read via
  `/api/model/default` at the project location) and is pinned explicitly on
  every phase session AND the watch subagent.
- Payload JSON must be built with `jq -n --rawfile` / `jq -n --arg` — never
  string-interpolated (phase text contains quotes and newlines).
