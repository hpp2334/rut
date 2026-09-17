---
name: Batch Plan Implementation
description: Execute a multi-phase/step plan by dispatching each phase to a fresh headless OpenCode session (via a subagent that dispatches and monitors with 60s polling), verifying the commit, then closing the session and moving to the next phase until the plan is done.
---

# Batch Plan Implementation

Drive an existing multi-phase/step plan to completion. Each phase is implemented
by a **fresh headless OpenCode session** created through the local API; a
**subagent** handles the dispatch + monitoring so the orchestrating session
(this one) stays free. Phases run strictly sequentially, one commit-ish per
phase.

## Rules

- One phase = one dedicated session = one subagent. Never batch phases.
- Never implement a phase in this session. You orchestrate and verify only.
- Poll status every **60 seconds**. Do not use blocking waits.
- On any phase failure, timeout, or stall: **stop the batch**, leave that
  session alive for inspection, and report to the user.
- Close (delete) a phase session only after its result is verified.
- If anything requires a decision (unclear plan, dirty tree, permission ask,
  failing tests), stop and ask the user instead of guessing.

## Workflow

### 0. Collect the plan

Identify the plan to execute, in this order:

1. The plan most recently produced in **this conversation** (e.g. by a plan
   agent or a previous discussion).
2. A plan file the user names.
3. Otherwise, look for obvious plan documents (`rfc/`, `docs/`, `plans/`,
   `*.plan.md`, TODO files) and ask the user to pick one.

Extract an ordered list of phases/steps. For each, capture:

- `n` — ordinal
- `title` — short name
- `detail` — the FULL text of that phase/step from the plan, verbatim
  (acceptance criteria, file paths, spec excerpts — everything)

If the plan text is ambiguous about phase boundaries or ordering, ask the user
before continuing.

### 1. Confirm with the user

Show the extracted list and get explicit approval before dispatching anything:

```
Batch plan execution — N phases
  1. <title> — <one-line summary>
  2. ...
Working dir: <abs path>    Base commit: <short hash>
Each phase: new headless session -> implement + commit -> poll 60s -> close.
Proceed?
```

### 2. Preconditions

```sh
git status --porcelain          # must be empty; if not, ask the user
git rev-parse HEAD              # record as $BASE
pwd                             # record as $PROJECT_DIR (absolute)
```

Store `$BASE` and `$PROJECT_DIR` — every subagent prompt needs them. Also
prepare a scratch dir for JSON payloads (payloads contain quotes/newlines;
never inline them into `--data`):

```sh
mkdir -p /tmp/opencode/batch-plan-impl
```

### 3. Dispatch each phase (loop for n = 1..N)

For phase `n`, spawn ONE subagent (subagent tool, `general` agent, foreground —
phases are sequential). The subagent prompt must be **fully self-contained**
(subagents start with no context). Use exactly this template, filling the
placeholders:

```text
You are a dispatch-and-monitor worker for phase <n> of a batch plan.
Work dir: <PROJECT_DIR>. All git commands must use: git -C <PROJECT_DIR> ...
Scratch dir for payload files: /tmp/opencode/batch-plan-impl

Do EXACTLY these steps, in order:

1. CREATE the implementation session:
   opencode api post /api/session \
     --data '{"title":"Phase <n>: <title>","location":{"directory":"<PROJECT_DIR>"}}'
   Extract the session id: jq '.data.id' — call it $SID.
   (location.directory is REQUIRED; without it the session lands elsewhere.)

2. BUILD the prompt payload file. Write the following verbatim text as the
   prompt (it is the phase the session must implement):
---8<--- PROMPT BEGIN ---8<---
You are implementing phase <n> of an approved plan in this repository.

<detail>

Rules:
- Implement ONLY this phase. Do not start or anticipate other phases.
- Work within this repo. Keep changes minimal and consistent with existing code.
- When the phase is done, commit ALL your changes with message:
  "phase(<n>): <title>"
  (include a short body listing what was done).
- Finish with a summary: what you changed, files touched, test/build results.
---8<--- PROMPT END ---8<---

   Write it to a file safely (never interpolate raw text into the command):
   jq -n --rawfile text /tmp/opencode/batch-plan-impl/phase<n>.txt '{text:$text}' \
     > /tmp/opencode/batch-plan-impl/phase<n>.json
   (create the .txt file first with the write tool or a heredoc)

3. DISPATCH it and record the admission timestamp:
   opencode api post /api/session/$SID/prompt \
     --data "$(cat /tmp/opencode/batch-plan-impl/phase<n>.json)"
   jq '.data.time.created'  — call it $SINCE (epoch ms).

4. MONITOR by polling every 60 seconds until done:
   for i in $(seq 1 240); do          # 240 polls = 4h hard cap
     sleep 60
     opencode api session.message.list \
       --param sessionID="$SID" --param order=desc --param limit=1 \
       | jq -e --argjson since "$SINCE" \
         '.data[0].type=="idle" and .data[0].time.created > $since' >/dev/null \
       && break
   done
   - The newest message being `idle` (after our prompt) means the loop finished.
   - Do NOT use /api/session/active — it does not track headless sessions.
   - Each `sleep 60` must be its own shell call so it fits command timeouts.

5. COLLECT the result:
   a) Final assistant summary text:
      opencode api session.message.list \
        --param sessionID="$SID" --param order=desc --param limit=15 \
      | jq -r '[.data[] | select(.type=="assistant")][0]
               | [.content[] | select(.type=="text") | .text] | join("\n")'
   b) idle outcome (should be "succeeded"):
      ... same list call ... | jq -r '.data[] | select(.type=="idle") | .outcome'
   c) Commit verification:
      git -C <PROJECT_DIR> log --oneline <BASE>..HEAD
      git -C <PROJECT_DIR> status --porcelain
   d) Timeout/stall detection: if the poll loop ends without idle, or no new
      commit appeared and the newest assistant message has not advanced for
      ~20 minutes, treat the phase as STALLED.

6. Do NOT delete the session. Do NOT modify the repository yourself.

7. RESPOND with exactly this report and nothing else:
   STATUS: DONE | FAILED | STALLED
   SESSION: $SID
   OUTCOME: <idle outcome or "timeout">
   COMMITS: <new commit hashes + subjects, or "none">
   DIRTY: <yes/no — git status --porcelain output>
   SUMMARY: <the session's final assistant text, max ~30 lines>
```

### 4. Review the subagent report

- `STATUS: DONE` + at least one commit + clean tree → phase succeeded:
  1. Close the session: `opencode api delete /api/session/$SID`
  2. Continue to the next phase (back to step 3).
- `STATUS: DONE` but no commit or dirty tree → treat as FAILED (ask the user:
  have the same session fix it, or stop?).
- `STATUS: FAILED` / `STALLED` → **stop the batch**. Keep the session alive.
  Report to the user: phase number, session id, summary, and how to inspect
  (session is still in the session list). Ask whether to retry the phase
  (new session, same prompt), skip, or abort the batch.

### 5. Final report

After the last phase (or on abort), summarize:

```
Batch plan complete: <k>/<N> phases done (base <BASE> -> HEAD <short hash>)
  1. <title>  ✓ <commit>  (session closed)
  2. <title>  ✓ <commit>  (session closed)
  3. <title>  ✗ FAILED — session ses_xxx kept for inspection
Overall diff stat: git -C . diff --stat <BASE>..HEAD
```

## Quick command reference

| Action | Command |
| --- | --- |
| Create session | `opencode api post /api/session --data '{"title":"...","location":{"directory":"<abs dir>"}}'` |
| Send prompt | `opencode api post /api/session/$SID/prompt --data "$(cat payload.json)"` |
| Poll newest message | `opencode api session.message.list --param sessionID=$SID --param order=desc --param limit=1` |
| Done when | newest msg `type=="idle"` and `time.created` > prompt admission time |
| Final summary text | list limit=15, first `assistant` msg, join its `content[]` where `type=="text"` |
| Close session | `opencode api delete /api/session/$SID` |
| Interrupt (if needed) | `opencode api post /api/session/$SID/interrupt` |

## Notes

- Sessions inherit the project's default model/agent from `opencode.jsonc`.
- If the project uses permission prompts, phase sessions may pause on asks;
  this surfaces as a stall. Handle permission decisions in the user-facing
  session, not by auto-approving inside the subagent.
- Payload JSON must be built with `jq -n --rawfile` / `jq -n --arg` — never
  string-interpolated (phase text contains quotes and newlines).
