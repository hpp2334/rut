// dev:channel — the todolist page behind a Cloudflare QUICK tunnel:
// a static serve of THIS directory (loader.js fetches the rut sources
// at runtime, so the whole tree is the page) with an ephemeral
// https://<random>.trycloudflare.com URL for the room. A quick tunnel
// needs no account and no config and the hostname is throwaway — right
// for a demo, never for anything durable.
//
// Zero npm deps, like everything in this example (the e2e gate's own
// law). Both children ride this process: the URL is fished out of
// cloudflared's connection log and printed LOUD, Ctrl+C tears down
// both, and either child dying tears down the other (no half-alive
// lanes).
import { spawn, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url)) + '/..';
const PORT = 8000;
const TUNNEL_URL_RE = /https:\/\/[a-z0-9-]+\.trycloudflare\.com/i;

const GLUE = [join(HERE, 'gen', 'web_host.js'), join(HERE, 'gen', 'web_host_bg.wasm')];

// preflight — the loader's manual build recipe, automated (the same
// steps tests/e2e-browser.mjs tier 1 runs when the artifact is missing):
// present glue = instant start; missing glue = a loud build right here,
// never a boot that dies in the browser in front of the room
if (GLUE.every(existsSync)) {
  console.log('preflight ok: gen/ glue present');
} else {
  console.log('gen/ glue missing — building (the manual recipe, loud):');
  const cargo = spawnSync(
    'cargo',
    ['build', '-p', 'todolist-web', '--target', 'wasm32-unknown-unknown', '--release'],
    { cwd: HERE, stdio: 'inherit' },
  );
  if (cargo.status !== 0) {
    console.error(`FAIL — cargo build exited ${cargo.status ?? '?'}`);
    process.exit(1);
  }
  const bindgen = spawnSync(
    'wasm-bindgen',
    ['--target', 'web', '--out-dir', 'gen', '--out-name', 'web_host',
     '../../target/wasm32-unknown-unknown/release/todolist_web.wasm'],
    { cwd: HERE, stdio: 'inherit' },
  );
  if (bindgen.status !== 0) {
    console.error(
      `FAIL — wasm-bindgen exited ${bindgen.status ?? '?'} — the CLI ` +
        `version must match the crate's wasm-bindgen (see Cargo.lock; ` +
        `wasm-pack's fetched install works).`,
    );
    process.exit(1);
  }
  for (const glue of GLUE) {
    if (!existsSync(glue)) {
      console.error(`FAIL — build claimed success but ${glue} is missing`);
      process.exit(1);
    }
  }
  console.log('preflight ok: gen/ glue built');
}

const devServer = spawn(
  'python3',
  ['-m', 'http.server', String(PORT)],
  { cwd: HERE, stdio: 'inherit' },
);
const tunnel = spawn(
  'cloudflared',
  // http2, not the quic default: quick tunnels ride UDP 7844 only in
  // quic mode, and restrictive NATs (the kind a demo room has) drop
  // it — the self-check then registers ONE half-alive connection the
  // edge never routes to (the empty-404 symptom). http2 rides TCP 443
  // like every other HTTPS flow, so it survives where quic cannot.
  ['tunnel', '--url', `http://localhost:${PORT}`],
  // cloudflared logs (including the URL) on stderr; keep stdout clear
  { stdio: ['ignore', 'ignore', 'pipe'] },
);

for (const [name, child] of [
  ['http.server', devServer],
  ['cloudflared', tunnel],
]) {
  child.on('error', (err) => {
    console.error(
      `could not spawn ${name}: ${err.message}` +
        (name === 'cloudflared' && err.code === 'ENOENT'
          ? ` — install it: curl -fL -o ~/.local/bin/cloudflared ` +
            `https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64`
          : ''),
    );
    teardown(1);
  });
}

// fish the quick-tunnel URL out of cloudflared's stderr and print it
// loud — on demo day nobody scrolls for the address
let stderrTail = '';
let announced = false;
tunnel.stderr.on('data', (chunk) => {
  stderrTail += chunk.toString();
  let newlineAt;
  while ((newlineAt = stderrTail.indexOf('\n')) !== -1) {
    const line = stderrTail.slice(0, newlineAt);
    stderrTail = stderrTail.slice(newlineAt + 1);
    process.stderr.write(`channel | ${line}\n`);
    const url = line.match(TUNNEL_URL_RE)?.[0];
    if (url && !announced) {
      announced = true;
      console.log(
        `\n` +
          `=======================================================\n` +
          `  CHANNEL IS UP — share this URL:\n` +
          `    ${url}\n` +
          `  (quick tunnel: throwaway hostname, lives while\n` +
          `   this process lives; local server: :${PORT})\n` +
          `=======================================================\n`,
      );
    }
  }
});

// cloudflared normally hands out the URL within seconds; if it hasn't,
// say so instead of letting the room stare at a silent terminal
const urlDeadline = setTimeout(() => {
  if (!announced && tunnel.exitCode === null) {
    console.error(
      `no trycloudflare URL after 30s — scroll the \`channel |\` ` +
        `lines above for cloudflared's actual complaint.`,
    );
  }
}, 30_000);

const children = [devServer, tunnel];
const alive = (child) => child.exitCode === null && child.signalCode === null;
let exiting = false;

function teardown(code) {
  if (exiting) return;
  exiting = true;
  clearTimeout(urlDeadline);
  for (const child of children) {
    if (alive(child)) child.kill('SIGTERM');
  }
  // the sticklers get SIGKILL; .unref keeps a hung child from pinning us
  setTimeout(() => {
    for (const child of children) {
      if (alive(child)) child.kill('SIGKILL');
    }
  }, 3_000).unref();
  const stillAlive = children.filter(alive);
  let dead = children.length - stillAlive.length;
  for (const child of stillAlive) {
    child.once('exit', () => {
      if (++dead === children.length) process.exit(code ?? 0);
    });
  }
  if (stillAlive.length === 0) process.exit(code ?? 0);
}

// either child dying tears down the other — a tunnel to a dead server
// (or a server nobody can reach) is exactly the half-alive lane this
// script exists to prevent
devServer.on('exit', (code) => {
  if (!exiting) {
    console.error(`http.server exited (code ${code}) — dropping the tunnel`);
    teardown(code ?? 1);
  }
});
tunnel.on('exit', (code) => {
  if (!exiting) {
    console.error(`cloudflared exited (code ${code}) — dropping the server`);
    teardown(code ?? 1);
  }
});

// Ctrl+C: the terminal already SIGINTs the whole foreground group;
// this makes the exit deliberate (and covers `kill <pid>` on the parent)
process.on('SIGINT', () => teardown(130));
process.on('SIGTERM', () => teardown(143));
