# AGENTS.md — notes for coding agents working in this repo

## Browser drives: use `agent-browser`, not hand-rolled WebDriver

When a task needs a REAL browser (driving the demo playground, the
05-todolist-web page, verifying a tunnel URL, any "open it and check"
acceptance):

1. **`agent-browser` is installed** (`/usr/local/bin/agent-browser`) and
   is the default. It is headless, needs no desktop app, and takes
   plain commands:

   ```sh
   agent-browser open <url>        # navigate
   agent-browser snapshot          # a11y tree with @eN refs (ALWAYS refetch after nav/changes)
   agent-browser click @e3         # click / fill @e2 "text" / press Enter / find "text"
   agent-browser screenshot out.png
   ```

2. The session's `browser.*` MCP tools (desktop bridge) may report
   `[browser.disconnected]` in headless/batch sessions. That is NOT a
   reason to fall back to hand-rolled WebDriver plumbing — `agent-browser`
   works there.

3. Last resorts, in order:
   - the repo's own gates: `cd demo && npm run smoke` (headless, no
     browser) and `node tests/e2e-browser.mjs` from
     `examples/05-todolist-web` (node tier + Firefox-WebDriver tier);
   - raw geckodriver + headless Firefox via WebDriver HTTP only when
     EXTENDING those gates, not for ad-hoc drives.

### Practical notes

- The wasm pages boot asynchronously: the playground probes
  `GET /rut.wasm`, the todolist page mounts into `#app` after the glue
  instantiates. Over a tunnel that is a few seconds — poll/snapshot
  again instead of declaring the page broken on the first empty tree.
- Live demo lanes (rf. the READMEs): `demo/scripts/dev-channel.mjs`
  and `examples/05-todolist-web/scripts/dev-channel.mjs` serve the page
  on localhost and print a throwaway
  `https://<random>.trycloudflare.com` URL; `agent-browser` can drive
  that public URL directly, which is the closest thing to what the
  room will see.
- Deployed target: `cd demo && npm run deploy` ships the playground to
  Cloudflare Pages (production from `master`):
  <https://playground.rut.hpp2334.com>.
