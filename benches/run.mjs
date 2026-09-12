#!/usr/bin/env node
// benches/run.mjs — cross-runtime benchmark runner.
//
// Compares rut against QuickJS (benches/.tools/qjs, vendored quickjs-ng)
// and V8 (the current Node.js binary) on the workloads in benches/workloads.
//
// For every runtime it measures end-to-end process wall time and peak RSS
// (via GNU /usr/bin/time, when available). For rut it additionally runs
// benches/probe (rut-bench-probe) to split compile / decode+verify / execute,
// and to read the VM-heap high-water mark (RFC 0039).
//
// Usage:
//   node benches/run.mjs [options]
//
// Options:
//   --runtime rut,node,qjs   runtimes to include (default: all available)
//   --workload sieve,nbody   workloads to include (default: all)
//   --repeats N              timed repetitions per runtime (default 3)
//   --warmup N               untimed warmups per runtime (default 1)
//   --probe-iters N          fresh-VM iterations for the rut probe (default 3)
//   --timeout SEC            per-run timeout (default 300)
//   --no-probe               skip the rut in-process probe
//   --no-build               do not build missing binaries
//   --quick                  1 rep, no warmup, 1 probe iter
//   --json PATH              write raw results as JSON
//   --md PATH                write a markdown report
//   --csv PATH               write cross-runtime CSV
//   --list                   list workloads and exit
//   -h, --help               this help
//
// Exit code: 0 on success, 1 if any runtime failed or checksums disagreed.

import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { arch, cpus, platform, release, tmpdir, totalmem } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const WORKLOADS_DIR = join(HERE, "workloads");
const EXPECTED_FILE = join(WORKLOADS_DIR, "expected.json");
const RESULTS_DIR = join(HERE, "results");
const RUT = join(ROOT, "target", "release", "rut");
const PROBE = join(ROOT, "target", "release", "rut-bench-probe");
const QJS = join(HERE, ".tools", "qjs");
const BUILD_QJS = join(HERE, "tools", "build-quickjs.sh");

const ALL_RUNTIMES = ["rut", "node", "qjs"];

// ---------------------------------------------------------------- options

function parseArgs(argv) {
  const opt = {
    runtimes: ALL_RUNTIMES.slice(),
    workloads: null,
    repeats: 3,
    warmup: 1,
    probeIters: 3,
    timeoutMs: 300_000,
    probe: true,
    build: true,
    json: null,
    md: null,
    csv: null,
    list: false,
  };
  const need = (i, name) => {
    if (i + 1 >= argv.length) fail(`${name} needs a value`);
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    switch (argv[i]) {
      case "--runtime":
        opt.runtimes = need(i, "--runtime").split(",").map((s) => s.trim()).filter(Boolean);
        i++;
        break;
      case "--workload":
        opt.workloads = need(i, "--workload").split(",").map((s) => s.trim()).filter(Boolean);
        i++;
        break;
      case "--repeats":
        opt.repeats = parseInt(need(i, "--repeats"), 10);
        i++;
        break;
      case "--warmup":
        opt.warmup = parseInt(need(i, "--warmup"), 10);
        i++;
        break;
      case "--probe-iters":
        opt.probeIters = parseInt(need(i, "--probe-iters"), 10);
        i++;
        break;
      case "--timeout":
        opt.timeoutMs = Math.round(parseFloat(need(i, "--timeout")) * 1000);
        i++;
        break;
      case "--no-probe":
        opt.probe = false;
        break;
      case "--quick":
        opt.repeats = 1;
        opt.warmup = 0;
        opt.probeIters = 1;
        break;
      case "--no-build":
        opt.build = false;
        break;
      case "--json":
        opt.json = need(i, "--json");
        i++;
        break;
      case "--md":
        opt.md = need(i, "--md");
        i++;
        break;
      case "--csv":
        opt.csv = need(i, "--csv");
        i++;
        break;
      case "--list":
        opt.list = true;
        break;
      case "-h":
      case "--help":
        opt.help = true;
        break;
      default:
        fail(`unknown argument: ${argv[i]}`);
    }
  }
  return opt;
}

function fail(msg) {
  console.error(`run.mjs: ${msg}`);
  process.exit(2);
}

function usage() {
  const src = readFileSync(fileURLToPath(import.meta.url), "utf8");
  const header = src.split("\n").slice(1, 31).map((l) => l.replace(/^\/\/ ?/, "")).join("\n");
  console.log(header);
}

// ------------------------------------------------------------- workloads

function discoverWorkloads() {
  return readdirSync(WORKLOADS_DIR)
    .filter((f) => f.endsWith(".rut"))
    .map((f) => basename(f, ".rut"))
    .sort();
}

// ------------------------------------------------------------ processes

const hasGnuTime =
  spawnSync("/usr/bin/time", ["--version"], { encoding: "utf8" }).status === 0;

let timeSeq = 0;

/// Run `cmd args...` once, returning wall time and peak RSS.
function timeRun(cmd, args, timeoutMs) {
  let wrappedCmd = cmd;
  let wrappedArgs = args;
  let rssFile = null;
  if (hasGnuTime) {
    rssFile = join(tmpdir(), `rutbench-${process.pid}-${timeSeq++}.rss`);
    wrappedCmd = "/usr/bin/time";
    wrappedArgs = ["-f", "%M", "-o", rssFile, "--", cmd, ...args];
  }
  const t0 = process.hrtime.bigint();
  const r = spawnSync(wrappedCmd, wrappedArgs, {
    encoding: "utf8",
    timeout: timeoutMs,
    maxBuffer: 64 * 1024 * 1024,
  });
  const wallMs = Number(process.hrtime.bigint() - t0) / 1e6;
  let peakRssKb = null;
  if (rssFile) {
    try {
      if (existsSync(rssFile)) {
        const txt = readFileSync(rssFile, "utf8").trim();
        const n = parseInt(txt, 10);
        if (Number.isFinite(n)) peakRssKb = n;
      }
    } catch {
      /* ignore */
    }
    rmSync(rssFile, { force: true });
  }
  return {
    wallMs,
    peakRssKb,
    stdout: r.stdout ?? "",
    stderr: r.stderr ?? "",
    status: r.status,
    signal: r.signal,
    timedOut: r.error && r.error.code === "ETIMEDOUT",
    error: r.error ? String(r.error.message ?? r.error) : null,
  };
}

function extractChecksum(stdout) {
  const m = stdout.match(/CHECKSUM\s+(.+)/);
  return m ? m[1].trim() : null;
}

// digits-only formatting helpers
const med = (xs) => {
  if (!xs.length) return NaN;
  const s = [...xs].sort((a, b) => a - b);
  const n = s.length;
  return n % 2 ? s[(n - 1) / 2] : (s[n / 2 - 1] + s[n / 2]) / 2;
};
const min = (xs) => (xs.length ? Math.min(...xs) : NaN);
const max = (xs) => (xs.length ? Math.max(...xs) : NaN);

// numeric checksum agreement: exact string, or numeric within tolerance
function checksumsAgree(a, b) {
  if (a == null || b == null) return a === b;
  if (a === b) return true;
  const x = Number(a);
  const y = Number(b);
  if (!Number.isFinite(x) || !Number.isFinite(y)) return false;
  return Math.abs(x - y) <= 1e-9 * Math.max(1, Math.abs(x), Math.abs(y));
}

// --------------------------------------------------------------- build

function ensureBuilt(opt) {
  const missing = [];
  if (!existsSync(RUT)) missing.push(RUT);
  if (opt.probe && !existsSync(PROBE)) missing.push(PROBE);
  if (missing.length && !opt.build) {
    fail(`missing binary: ${missing[0]} (build with: cargo build --release -p rut-cli -p rut-bench-probe)`);
  }
  if (missing.length) {
    console.error("building rut + probe (cargo build --release) …");
    const r = spawnSync(
      "cargo",
      ["build", "--release", "-p", "rut-cli", "-p", "rut-bench-probe"],
      { cwd: ROOT, stdio: "inherit" },
    );
    if (r.status !== 0) fail("cargo build failed");
  }
}

function ensureQjs(opt) {
  if (existsSync(QJS)) return true;
  if (!opt.build) return false;
  console.error("building vendored quickjs-ng (benches/tools/build-quickjs.sh) …");
  const r = spawnSync("bash", [BUILD_QJS], { stdio: "inherit" });
  return r.status === 0 && existsSync(QJS);
}

function runtimeSpec(name, opt) {
  switch (name) {
    case "rut":
      return {
        name,
        ext: ".rut",
        available: existsSync(RUT),
        argv: (file) => [RUT, "run", file],
      };
    case "node":
      return {
        name,
        ext: ".js",
        available: true,
        argv: (file) => [process.execPath, file],
      };
    case "qjs":
      return {
        name,
        ext: ".js",
        available: ensureQjs(opt),
        argv: (file) => [QJS, file],
      };
    default:
      fail(`unknown runtime: ${name}`);
  }
}

// ---------------------------------------------------------------- run

function measureRuntime(spec, file, opt) {
  const cmd = spec.argv(file);
  const warm = [];
  for (let i = 0; i < opt.warmup; i++) {
    const r = timeRun(cmd[0], cmd.slice(1), opt.timeoutMs);
    warm.push(r);
    if (r.timedOut || r.status !== 0) break;
  }
  const runs = [];
  for (let i = 0; i < opt.repeats; i++) {
    const r = timeRun(cmd[0], cmd.slice(1), opt.timeoutMs);
    runs.push(r);
    if (r.timedOut || r.status !== 0) break;
  }
  const sample = runs.find((r) => r.stdout) ?? warm.find((r) => r.stdout);
  const failures = [...warm, ...runs].filter((r) => r.timedOut || r.status !== 0);
  const walls = runs.filter((r) => !r.timedOut && r.status === 0).map((r) => r.wallMs);
  const rss = runs.filter((r) => r.peakRssKb != null).map((r) => r.peakRssKb);
  return {
    runtime: spec.name,
    checksum: sample ? extractChecksum(sample.stdout) : null,
    wallMedianMs: med(walls),
    wallMinMs: min(walls),
    peakRssKb: max(rss), // the worst observed per-run peak RSS
    ran: runs.length,
    ok: failures.length === 0 && walls.length > 0,
    error: failures.length
      ? failures[0].timedOut
        ? "timeout"
        : `exit ${failures[0].status}${failures[0].error ? `: ${failures[0].error}` : ""}`
      : null,
    stderr: failures.length ? failures[0].stderr.split("\n").slice(0, 4).join(" | ") : "",
  };
}

function runProbe(file, opt) {
  const r = spawnSync(PROBE, ["--workload", file, "--iters", String(opt.probeIters)], {
    encoding: "utf8",
    timeout: opt.timeoutMs,
    maxBuffer: 16 * 1024 * 1024,
  });
  if (r.status !== 0 && !r.stdout) {
    return { error: r.stderr?.trim() || `exit ${r.status}` };
  }
  try {
    const line = r.stdout.trim().split("\n").filter(Boolean).pop();
    return JSON.parse(line);
  } catch (e) {
    return { error: `bad probe output: ${e.message}` };
  }
}

// ------------------------------------------------------------- reports

function fmtMs(v) {
  if (!Number.isFinite(v)) return "—";
  if (v >= 1000) return `${(v / 1000).toFixed(2)} s`;
  if (v >= 10) return `${v.toFixed(1)} ms`;
  return `${v.toFixed(3)} ms`;
}
function fmtMb(kb) {
  if (kb == null || !Number.isFinite(kb)) return "—";
  return `${(kb / 1024).toFixed(1)} MB`;
}
function fmtBytes(b) {
  if (b == null || !Number.isFinite(b)) return "—";
  if (b >= 1024 * 1024) return `${(b / 1048576).toFixed(2)} MB`;
  if (b >= 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${b} B`;
}

function pad(s, w, right = false) {
  s = String(s);
  return right ? s.padStart(w) : s.padEnd(w);
}

function renderCrossTable(rows) {
  const head = ["workload", "runtime", "checksum", "wall median", "wall min", "peak RSS", "ref", "ok"];
  const body = rows.map((r) => [
    r.workload,
    r.runtime,
    r.checksum ?? "—",
    fmtMs(r.wallMedianMs),
    fmtMs(r.wallMinMs),
    fmtMb(r.peakRssKb),
    r.refOk == null ? "—" : r.refOk ? "yes" : "MISMATCH",
    r.ok ? (r.agree ? "yes" : "MISMATCH") : `FAIL (${r.error})`,
  ]);
  const widths = head.map((h, i) =>
    Math.max(h.length, ...body.map((b) => String(b[i]).length)),
  );
  const line = (cells) =>
    cells.map((c, i) => pad(c, widths[i], i >= 2)).join("  ");
  const sep = widths.map((w) => "-".repeat(w)).join("  ");
  return [line(head), sep, ...body.map(line)].join("\n");
}

function renderRutTable(rows) {
  const head = ["workload", "compile", "verify", "exec median", "fuel", "VM heap peak", "trap"];
  const body = rows.map((r) => [
    r.workload,
    fmtMs(r.compile_ms),
    fmtMs(r.verify_ms),
    fmtMs(r.exec_median_ms),
    r.fuel != null ? String(r.fuel) : "—",
    fmtBytes(r.heap_peak_bytes),
    r.trapped ?? "—",
  ]);
  const widths = head.map((h, i) =>
    Math.max(h.length, ...body.map((b) => String(b[i]).length)),
  );
  const line = (cells) => cells.map((c, i) => pad(c, widths[i], i >= 1)).join("  ");
  const sep = widths.map((w) => "-".repeat(w)).join("  ");
  return [line(head), sep, ...body.map(line)].join("\n");
}

// ---------------------------------------------------------------- main

const opt = parseArgs(process.argv.slice(2));
if (opt.help) {
  usage();
  process.exit(0);
}

let workloadNames = discoverWorkloads();
if (opt.list) {
  console.log(workloadNames.join("\n"));
  process.exit(0);
}
if (opt.workloads) {
  const want = new Set(opt.workloads);
  workloadNames = workloadNames.filter((w) => want.has(w));
  const missing = [...want].filter((w) => !workloadNames.includes(w));
  if (missing.length) fail(`unknown workload(s): ${missing.join(", ")}`);
}
if (!workloadNames.length) fail("no workloads selected");

ensureBuilt(opt);

const runtimes = opt.runtimes
  .map((n) => runtimeSpec(n, opt))
  .filter((s) => {
    if (!s.available) console.error(`run.mjs: runtime '${s.name}' not available — skipping`);
    return s.available;
  });
if (!runtimes.length) fail("no runtimes available");

if (!hasGnuTime) {
  console.error("run.mjs: GNU /usr/bin/time not found — peak RSS will be unavailable");
}

const crossRows = [];
const rutRows = [];
const perWorkload = [];

// Optional canonical reference checksums (benches/workloads/expected.json).
// Cross-runtime agreement alone cannot catch a bug shared by all three
// implementations; this catches it.
let expectedRefs = {};
if (existsSync(EXPECTED_FILE)) {
  expectedRefs = JSON.parse(readFileSync(EXPECTED_FILE, "utf8"));
}

for (const name of workloadNames) {
  const rutFile = join(WORKLOADS_DIR, `${name}.rut`);
  const jsFile = join(WORKLOADS_DIR, `${name}.js`);
  const entry = { workload: name, runtimes: [], checksums: {} };
  const checked = [];
  for (const spec of runtimes) {
    const file = spec.ext === ".js" ? jsFile : rutFile;
    if (spec.ext === ".js" && !existsSync(file)) {
      console.error(`run.mjs: ${name}: missing ${file} — skipping ${spec.name}`);
      continue;
    }
    process.stderr.write(`run: ${name} [${spec.name}] … `);
    const m = measureRuntime(spec, file, opt);
    if (!m.ok) {
      process.stderr.write(`FAIL (${m.error})\n`);
      if (m.stderr) process.stderr.write(`      ${m.stderr}\n`);
    } else {
      process.stderr.write(`${fmtMs(m.wallMedianMs)}  ${fmtMb(m.peakRssKb)}\n`);
    }
    const agree =
      m.checksum == null || checked.every((c) => checksumsAgree(m.checksum, c));
    if (m.checksum != null) checked.push(m.checksum);
    const ref = expectedRefs[name] ?? null;
    const refOk =
      ref == null || m.checksum == null
        ? null
        : checksumsAgree(m.checksum, String(ref));
    entry.runtimes.push({ ...m, agree, refOk });
    entry.checksums[spec.name] = m.checksum;
    crossRows.push({ workload: name, ...m, agree, refOk });
  }
  if (opt.probe && runtimes.some((r) => r.name === "rut")) {
    process.stderr.write(`probe: ${name} [rut] … `);
    const p = runProbe(rutFile, opt);
    if (p.error) {
      process.stderr.write(`FAIL (${p.error})\n`);
    } else {
      process.stderr.write(
        `compile ${fmtMs(p.compile_ms)}  exec ${fmtMs(p.exec_median_ms)}  heap ${fmtBytes(p.heap_peak_bytes)}\n`,
      );
      rutRows.push({ ...p, workload: name });
    }
    entry.probe = p;
  }
  perWorkload.push(entry);
}

const mismatches = crossRows.filter((r) => !r.agree && r.checksum != null);
const refMismatches = crossRows.filter((r) => r.refOk === false);

console.log("\n=== cross-runtime (end-to-end process) ===\n");
console.log(renderCrossTable(crossRows));

if (rutRows.length) {
  console.log("\n=== rut in-process (rut-bench-probe) ===\n");
  console.log(renderRutTable(rutRows));
}

console.log("\n=== notes ===");
console.log(
  `* rut is a research bytecode interpreter; V8 (node ${process.version}) JITs and QuickJS is an optimizing interpreter.`,
);
console.log(
  "* wall time includes parse/compile, verifier, and execution for every runtime, exactly as each is normally invoked.",
);
console.log(
  "* peak RSS is the whole-process maximum resident set (GNU time %M); it includes each runtime's baseline — subtract the 'empty' row.",
);
console.log(
  "* VM heap peak is rut's self-accounted live-cell high-water (RFC 0039), not process memory.",
);
if (mismatches.length) {
  console.log(`\n!! cross-runtime checksum mismatch in: ${mismatches.map((m) => `${m.workload}/${m.runtime}`).join(", ")}`);
}
if (refMismatches.length) {
  console.log(`\n!! reference checksum mismatch in: ${refMismatches.map((m) => `${m.workload}/${m.runtime}`).join(", ")}`);
}

if (opt.json || opt.md || opt.csv) mkdirSync(RESULTS_DIR, { recursive: true });

const meta = {
  date: new Date().toISOString(),
  node: process.version,
  platform: `${platform()} ${release()} ${arch()}`,
  cpu: cpus()?.[0]?.model ?? "unknown",
  cpuCount: cpus()?.length ?? 0,
  totalMemBytes: totalmem(),
  hasGnuTime,
  options: opt,
};
const payload = { meta, cross: crossRows, rut: rutRows, workloads: perWorkload };

if (opt.json) {
  writeFileSync(opt.json, JSON.stringify(payload, null, 2));
  console.log(`\njson: ${opt.json}`);
}
if (opt.md) {
  const md = [
    `# rut vs QuickJS vs V8 — benchmark report`,
    ``,
    `- Date: ${meta.date}`,
    `- Host: ${meta.platform}, ${meta.cpu} x${meta.cpuCount}, ${(meta.totalMemBytes / 1073741824).toFixed(1)} GiB`,
    `- Node: ${meta.node}`,
    `- Runtimes: ${runtimes.map((r) => r.name).join(", ")}`,
    `- Repeats: ${opt.repeats}, warmup: ${opt.warmup}, probe iters: ${opt.probeIters}`,
    ``,
    `## Cross-runtime (end-to-end process)`,
    ``,
    "```",
    renderCrossTable(crossRows),
    "```",
  ];
  if (rutRows.length) {
    md.push(``, `## rut in-process`, ``, "```", renderRutTable(rutRows), "```");
  }
  if (mismatches.length) {
    md.push(``, `**Cross-runtime checksum mismatches:** ${mismatches.map((m) => `${m.workload}/${m.runtime}`).join(", ")}`);
  }
  if (refMismatches.length) {
    md.push(``, `**Reference checksum mismatches:** ${refMismatches.map((m) => `${m.workload}/${m.runtime}`).join(", ")}`);
  }
  writeFileSync(opt.md, md.join("\n") + "\n");
  console.log(`md:   ${opt.md}`);
}
if (opt.csv) {
  const lines = ["workload,runtime,checksum,wall_median_ms,wall_min_ms,peak_rss_kb,ok"];
  for (const r of crossRows) {
    lines.push(
      [
        r.workload,
        r.runtime,
        `"${r.checksum ?? ""}"`,
        Number.isFinite(r.wallMedianMs) ? r.wallMedianMs.toFixed(3) : "",
        Number.isFinite(r.wallMinMs) ? r.wallMinMs.toFixed(3) : "",
        r.peakRssKb ?? "",
        r.ok ? (r.agree ? "yes" : "mismatch") : "fail",
      ].join(","),
    );
  }
  writeFileSync(opt.csv, lines.join("\n") + "\n");
  console.log(`csv:  ${opt.csv}`);
}

process.exit(mismatches.length || refMismatches.length ? 1 : 0);
