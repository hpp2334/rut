// dev:channel — the demo dev server behind a Cloudflare QUICK tunnel:
// `rspack serve` stays on localhost:8080 while cloudflared publishes an
// ephemeral https://<random>.trycloudflare.com URL for the room. A
// quick tunnel needs no account and no config and the hostname is
// throwaway — right for a demo, never for anything durable.
//
// Both children ride this process: the URL is fished out of
// cloudflared's connection log and printed LOUD, Ctrl+C tears down
// both, and either child dying tears down the other (no half-alive
// lanes). npm_lifecycle_event is inherited by the spawned server, so
// rspack.config.ts keeps the channel in the dev lane (refresh + HMR)
// with the tunnel's allowedHosts exception.
import { spawn } from 'node:child_process';

const PORT = 8080;
const TUNNEL_URL_RE = /https:\/\/[a-z0-9-]+\.trycloudflare\.com/i;

const devServer = spawn('rspack', ['serve'], { stdio: 'inherit' });
const tunnel = spawn(
  'cloudflared',
  [
    'tunnel',
    // http2, not the quic default: quick tunnels ride UDP 7844 only in
    // quic mode, and restrictive NATs (the kind a demo room has) drop
    // it — the self-check then registers ONE half-alive connection the
    // edge never routes to (the empty-404 symptom). http2 rides TCP 443
    // like every other HTTPS flow, so it survives where quic cannot.
    '--protocol',
    'http2',
    // a config file of our own (empty): cloudflared picks up
    // ~/.cloudflared/config.yml otherwise, and ANY leftover ingress
    // there — with its http_status:404 catch-all — silently WINS over
    // --url. The symptom is cruel: the tunnel registers, the URL is
    // served, and every request is answered 404 by cloudflared itself
    // (originService=http_status:404 in the debug log). /dev/null is
    // empty, so --url is the whole ingress again.
    '--config',
    '/dev/null',
    '--url',
    `http://localhost:${PORT}`,
  ],
  // cloudflared logs (including the URL) on stderr; keep stdout clear
  { stdio: ['ignore', 'ignore', 'pipe'] },
);

for (const [name, child] of [
  ['rspack serve', devServer],
  ['cloudflared', tunnel],
]) {
  child.on('error', (err) => {
    console.error(
      `could not spawn ${name}: ${err.message}` +
        (err.code === 'ENOENT'
          ? ` — is it installed and on PATH? (cloudflared: ` +
            `https://developers.cloudflare.com/cloudflare/one-page-docs/cloudflare-one/connections/connect-networks/downloads/ ` +
            `or curl -fL -o ~/.local/bin/cloudflared ` +
            `https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64)`
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
    console.error(`rspack serve exited (code ${code}) — dropping the tunnel`);
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
