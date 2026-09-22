// THE SMOKE (the phase 1 gate, extended in phase 2) — headless, no
// browser, demo-owned.
//
// Drives the SAME RutApi the React app uses (the bundled
// src/runner.ts + src/cases.ts + src/verify.ts + src/examples —
// dist-smoke/rut-api.cjs) against the SHIPPED public/rut.wasm:
//   1. the runner law: a missing/invalid artifact boots to mode
//      "error" naming the exact `npm run build:wasm` command; the
//      error runner has no working methods — no silent anything;
//   2. every prepared case (8 inline + 17 classics) REALLY compiles
//      and runs through the wasm engine at the pinned default budget
//      and verifies GREEN against its sidecar;
//   3. the resume is REAL (survey D5): the parked frame continues —
//      zero-budget resume re-traps at the parked pc, +16M resume
//      carries tick 2000000 (a re-run would print tick 1000000
//      again), fuelUsed is cumulative across legs;
//   4. a non-rut source gets real diagnostics and no binary — the
//      preview-era fake clean compile is dead;
//   5. THE GREP GATE (phase 2, survey §2): no dead-surface shapes
//      anywhere under demo/src — dataclass-as-keyword, `where`
//      clauses, `Ptr<`, `Hashable`, the removed `mapset` pkg, postfix
//      `?T` — scanned over the WHOLE tree, comments included,
//      whitelist nothing (the todolist app-law precedent).
//
// Exit 0 = every case real and verified. Any failure exits 1 LOUD.
'use strict';

import { createRequire } from 'node:module';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join, dirname } from 'node:path';

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));

const DEMO = join(here, '..');
const BUNDLE = join(DEMO, 'dist-smoke', 'rut-api.cjs');
const ARTIFACT = join(DEMO, 'public', 'rut.wasm');
const COMMAND = 'npm run build:wasm';
const DEFAULT_BUDGET = { fuel: 10_000_000, heapBytes: 4 * 1024 * 1024 };

let failures = 0;
let passes = 0;

function check(cond, label, detail) {
  if (cond) {
    passes += 1;
    console.log(`  ok  ${label}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${label}${detail ? `\n      ${detail}` : ''}`);
  }
}

function linesEqual(a, b) {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Node Buffers pool their ArrayBuffer — hand boot an EXACT copy */
function exactAB(buf) {
  return buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength);
}

const api = require(BUNDLE);

// ---- 1. the runner law ----

console.log('\n[1] the runner law: missing/invalid artifact => mode "error"');

{
  const missing = await api.Runner.boot(async () => {
    const e = new Error('not found');
    throw e;
  });
  check(missing.state.mode === 'error', 'missing artifact boots to error mode', missing.state.mode);
  check(
    missing.state.banner.includes(COMMAND),
    'the error names the exact build command',
    missing.state.banner,
  );
  check(!missing.isLive, 'the error runner is not live');
  let threw = false;
  try {
    missing.compile('pub fn main() {}');
  } catch {
    threw = true;
  }
  check(threw, 'an error runner refuses compile — no silent fallback');
  threw = false;
  try {
    missing.run(new Uint8Array(0), api.DEFAULT_BUDGET);
  } catch {
    threw = true;
  }
  check(threw, 'an error runner refuses run — no silent fallback');
}

{
  const garbage = await api.Runner.boot(async () => new Uint8Array([0, 1, 2, 3]));
  check(garbage.state.mode === 'error', 'a non-wasm artifact boots to error mode');
  check(garbage.state.banner.includes(COMMAND), 'the invalid-artifact error names the command');
}

{
  // an artifact truncated mid-magic cannot instantiate — boot must turn
  // the throw into the error panel, never a half-working page
  const stub = await api.Runner.boot(async () =>
    exactAB(readFileSync(ARTIFACT).subarray(0, 64)),
  );
  check(stub.state.mode === 'error', 'a truncated artifact boots to error mode');
  check(stub.state.banner.includes(COMMAND), 'the truncated-artifact error names the command');
}

// ---- 2. the real artifact: every case really runs and verifies green ----

console.log('\n[2] every prepared case: REAL run, verified against its sidecar');

const bytes = exactAB(readFileSync(ARTIFACT));
const runner = await api.Runner.boot(async () => bytes);
check(runner.state.mode === 'wasm', 'the artifact boots to wasm mode', runner.state.banner);
check(runner.isLive, 'the wasm runner is live');

const all = [...api.CASES, ...api.EXAMPLES];
check(all.length === 25, 'the full corpus is present (8 inline + 17 classics)', String(all.length));

for (const c of all) {
  runner.dropFrame();
  const compiled = runner.compile(c.source);
  check(
    compiled.diags.length === 0,
    `${c.id}: compiles clean`,
    JSON.stringify(compiled.diags?.map((d) => d.msg)),
  );
  check(Boolean(compiled.binary), `${c.id}: binary emitted`);
  check(
    Boolean(compiled.ast),
    `${c.id}: structured AST present`,
  );
  check(
    typeof compiled.irDump === 'string' &&
      compiled.irDump.length > 0 &&
      !compiled.irDump.includes('requires the wasm build'),
    `${c.id}: real IR dump (no placeholder)`,
  );
  const res = runner.run(compiled.binary, api.DEFAULT_BUDGET);
  const v = api.verifyAgainstExpected(res, c.expected, api.DEFAULT_BUDGET.fuel);
  check(
    v.ok,
    `${c.id}: REAL run matches its sidecar`,
    JSON.stringify({ got: api.renderedLines(res), expected: c.expected }),
  );
}

// ---- 3. the resume is REAL ----

console.log('\n[3] the resume: the parked frame CONTINUES — never a re-run');

const fuel = api.CASES.find((c) => c.id === 'fuel-demo');
runner.dropFrame();
{
  const compiled = runner.compile(fuel.source);
  // leg 1 at 12M: one tick (~10M ops to reach i=1M), then parked
  const leg1 = runner.run(compiled.binary, { fuel: 12_000_000, heapBytes: DEFAULT_BUDGET.heapBytes });
  check(leg1.parked === true, 'the frame parks on OutOfFuel (parked:true in the envelope)');
  check(
    linesEqual(leg1.output, ['tick 1000000']),
    'leg 1 output is the one tick',
    JSON.stringify(leg1.output),
  );
  check(leg1.trap === 'OutOfFuel', 'leg 1 traps OutOfFuel');

  // zero-budget resume: re-traps at the parked pc — same output, same
  // spent counter. A restart cannot burn 12M ops on a 0 budget.
  const zero = runner.resume(0);
  check(zero.parked === true, 'resume(0) parks again at the same pc');
  check(
    linesEqual(zero.output, ['tick 1000000']),
    'resume(0) prints nothing new — the frame is where it was parked',
    JSON.stringify(zero.output),
  );
  check(
    zero.fuelUsed >= leg1.fuelUsed && zero.fuelUsed < leg1.fuelUsed + 1_000,
    `resume(0) keeps the cumulative counter (${zero.fuelUsed} vs ${leg1.fuelUsed})`,
  );

  // +16M: the frame CONTINUES — i is still ~1.2M, so the next tick is
  // tick 2000000; a re-run from scratch would print tick 1000000 again
  const leg2 = runner.resume(16_000_000);
  check(
    linesEqual(leg2.output, ['tick 1000000', 'tick 2000000']),
    'resume(+16M) CONTINUES: tick 2000000 — no restarted tick 1000000',
    JSON.stringify(leg2.output),
  );
  check(!leg2.output.some((l) => l === 'tick 3000000'), '16M more ops cannot reach tick 3000000');
  check(leg2.trap === 'OutOfFuel' && leg2.parked === true, 'the loop parks again (it is infinite)');
  check(
    leg2.fuelUsed > 26_000_000,
    `fuelUsed is cumulative across legs (${leg2.fuelUsed}) — a fresh 16M run can never report that`,
  );

  // the accumulated verify vs the sidecar is a diff BY DESIGN (the
  // sidecar pins the DEFAULT budget) — and says so
  const v = api.verifyAgainstExpected(leg2, fuel.expected, 28_000_000);
  check(!v.ok, 'the accumulated output diffs the default-budget sidecar by design');
  check(
    v.rows.some((r) => r.expected === 'Trap::OutOfFuel'),
    'the diff pairs the trap line with its sidecar line',
  );

  // drop retires the frame; a further resume is LOUD
  runner.dropFrame();
  const after = runner.resume(1_000_000);
  check(
    typeof after.trap === 'string' && after.trap.includes('no parked frame'),
    'resume with no frame is loud',
    String(after.trap),
  );
  check(after.parked === false, 'the loud resume parks nothing');
}

// ---- 4. compile diags are real ----

console.log('\n[4] a non-rut source gets real diagnostics — no fake clean compile');

{
  const bad = runner.compile('this is not rut at all');
  check(bad.diags.length > 0, 'diagnostics are real', JSON.stringify(bad.diags?.map((d) => d.msg)));
  check(bad.binary === undefined, 'no binary for a source that does not parse');
}

// ---- 5. the dead-surface grep gate (phase 2, survey §2) ----

console.log('\n[5] the grep gate: no dead-surface shapes anywhere under demo/src');

{
  // the audit's removals, as exact shapes. The scan covers EVERY file
  // under demo/src — comments included, whitelist nothing: a comment
  // that teaches a dead spelling is the same dishonesty as code that
  // uses it. `mapset` is word-bounded so the mounted `nmapset` lane
  // (its replacement) passes; the postfix-`?` shape matches only
  // type position (`i32?`, `Node?)` — word char/`]`, then `?`, then a
  // type follower — so prefix `?T`, optional chaining, and ternaries
  // never trip it.
  const DEAD_SHAPES = [
    ['dataclass (the dead record keyword — records are spelled `struct`)', /\bdataclass/],
    ['`where` clause (removed by RFC 0043 — inline `requires`)', /\bwhere\b/],
    ['`Ptr<` (the removed pointer type — the shape is the nullable `?T`)', /Ptr</],
    ['`Hashable` (the removed trait — keys are admitted by the union bound)', /\bHashable\b/],
    ['`mapset` (the removed pkg — the lane is the mounted nmapset)', /\bmapset\b/],
    ['postfix `?T` (the nullable is prefix-only, RFC 0044 §2)', /[A-Za-z0-9_\]]\?(?=[;,=>)]|\s*$)/],
  ];

  function walk(dir) {
    const out = [];
    for (const entry of readdirSync(dir)) {
      const p = join(dir, entry);
      if (statSync(p).isDirectory()) {
        out.push(...walk(p));
      } else {
        out.push(p);
      }
    }
    return out;
  }

  const files = walk(join(DEMO, 'src')).sort();
  check(files.length >= 40, `the scan covered ${files.length} files under demo/src`, String(files.length));

  let hits = 0;
  for (const file of files) {
    const text = readFileSync(file, 'utf8');
    const rel = file.slice(DEMO.length + 1);
    for (const [label, re] of DEAD_SHAPES) {
      const lines = text.split('\n');
      for (let i = 0; i < lines.length; i++) {
        if (re.test(lines[i])) {
          hits += 1;
          console.error(`FAIL  ${rel}:${i + 1}: ${label}\n      ${lines[i].trim()}`);
        }
      }
    }
  }
  check(hits === 0, 'no dead-surface shapes in demo/src', `${hits} hits`);
}

// ---- verdict ----

console.log(`\nsmoke: ${passes} passed, ${failures} failed`);
if (failures > 0) {
  console.error('THE SMOKE IS RED — a case that does not really run and verify is not done.');
  process.exit(1);
}
console.log('THE SMOKE IS GREEN — every case really ran and verified; the resume really resumed.');
