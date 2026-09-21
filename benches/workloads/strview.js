// strview.js — the JS twin of strview/main.rut (the strings-round1
// phase-2 row): the nmapset-str six-phase churn with keys carved as
// fixed-width 6-char windows of ONE generated parent string, on
// `new Map()` keyed by `substring` — the materialized computation the
// rut row crosses as borrowed `(parent, i*6, 6)` byte windows. Same
// phases, same counters, same prime-weight formula as nmapset-str.js;
// the checksum therefore lands on the SAME value as the twin's pin
// (the formula reads counters + the value sum only — key content is
// invisible to it), and the three-runtime agreement is the gate.
// Scale: n = 50000 entries, parent = 600000 chars.

function buildParent(n) {
  let p = "";
  for (let i = 0; i < 2 * n; i++) {
    p += String(100000 + i);
  }
  return p;
}

function churn(p, n) {
  const m = new Map();
  // build — every key is new
  let added = 0;
  for (let i = 0; i < n; i++) {
    const k = p.substring(i * 6, i * 6 + 6);
    if (!m.has(k)) added++;
    m.set(k, i);
  }
  // replace — every key is present
  let replaced = 0;
  for (let i = 0; i < n; i++) {
    const k = p.substring(i * 6, i * 6 + 6);
    if (m.has(k)) replaced++;
    m.set(k, i + 3);
  }
  // hit lookups — sum the stored values
  let sum = 0;
  let hits = 0;
  for (let i = 0; i < n; i++) {
    const q = m.get(p.substring(i * 6, i * 6 + 6));
    if (q !== undefined) {
      hits++;
      sum = (sum + q) | 0;
    }
  }
  // miss lookups — slots n..2n are never inserted
  let misses = 0;
  for (let i = 0; i < n; i++) {
    if (m.get(p.substring((n + i) * 6, (n + i) * 6 + 6)) === undefined) misses++;
  }
  // remove every third key
  let removed = 0;
  for (let i = 0; i < n; i += 3) {
    if (m.delete(p.substring(i * 6, i * 6 + 6))) removed++;
  }
  // full re-scan: two thirds survive
  let present = 0;
  for (let i = 0; i < n; i++) {
    if (m.has(p.substring(i * 6, i * 6 + 6))) present++;
  }
  // re-add the removed keys under negated values
  let added2 = 0;
  for (let i = 0; i < n; i += 3) {
    const k = p.substring(i * 6, i * 6 + 6);
    if (!m.has(k)) added2++;
    m.set(k, -i);
  }
  let c = sum;
  c = (c + added * 31) | 0;
  c = (c + replaced * 37) | 0;
  c = (c + hits * 41) | 0;
  c = (c + misses * 43) | 0;
  c = (c + removed * 47) | 0;
  c = (c + present * 53) | 0;
  c = (c + added2 * 59) | 0;
  c = (c + m.size * 61) | 0;
  return c;
}

const n = 50000;
console.log("CHECKSUM " + churn(buildParent(n), n));
