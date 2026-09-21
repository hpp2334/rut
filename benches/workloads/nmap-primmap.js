// nmap-primmap.js — the JS twin of nmap-primmap/main.rut (the
// nmapset-round3 phase-2 row): the identical i32 churn on `new Map()`
// with integer keys — the same algorithm as nmapset-int.js, since the
// rut row changes only the VALUE STORAGE (native val column instead of
// a rut-side sidecar), never the op sequence. Same work, same checksum.
// Scale: n = 100000 entries.
// Adaptation: JS Map has no insert-or-replace primitive, so the
// add/replace split costs one extra `has` probe per put compared with
// rut's `put` -> bool.

const GOLD = -1640531535; // 0x9E3779B1 read as i32

function churn(n) {
  const m = new Map();
  // build — every key is new
  let added = 0;
  for (let i = 0; i < n; i++) {
    const k = (i * GOLD) | 0;
    if (!m.has(k)) added++;
    m.set(k, i);
  }
  // replace — every key is present
  let replaced = 0;
  for (let i = 0; i < n; i++) {
    const k = (i * GOLD) | 0;
    if (m.has(k)) replaced++;
    m.set(k, i + 7);
  }
  // hit lookups — sum the stored values
  let sum = 0;
  let hits = 0;
  for (let i = 0; i < n; i++) {
    const p = m.get((i * GOLD) | 0);
    if (p !== undefined) {
      hits++;
      sum = (sum + p) | 0;
    }
  }
  // miss lookups — the second half of the key space is absent
  let misses = 0;
  for (let i = 0; i < n; i++) {
    if (!m.has(((i + n) * GOLD) | 0)) misses++;
  }
  // remove the even keys
  let removed = 0;
  for (let i = 0; i < n; i += 2) {
    if (m.delete((i * GOLD) | 0)) removed++;
  }
  // full re-scan: the odds survive, the evens do not
  let present = 0;
  for (let i = 0; i < n; i++) {
    if (m.has((i * GOLD) | 0)) present++;
  }
  // re-add the evens under negated values
  let added2 = 0;
  for (let i = 0; i < n; i += 2) {
    const k = (i * GOLD) | 0;
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

console.log("CHECKSUM " + churn(100000));
