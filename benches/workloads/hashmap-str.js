// hashmap-str.js — must match hashmap-str/main.rut: the identical
// str-keyed churn on `new Map()` with string keys. Scale: n = 50000
// entries. Adaptation: JS Map has no insert-or-replace primitive, so
// the add/replace split costs one extra `has` probe per put compared
// with rut's `put` -> bool.

function churn(n) {
  const m = new Map();
  // build — every key is new
  let added = 0;
  for (let i = 0; i < n; i++) {
    const k = `k${i}`;
    if (!m.has(k)) added++;
    m.set(k, i);
  }
  // replace — every key is present
  let replaced = 0;
  for (let i = 0; i < n; i++) {
    const k = `k${i}`;
    if (m.has(k)) replaced++;
    m.set(k, i + 3);
  }
  // hit lookups — sum the stored values
  let sum = 0;
  let hits = 0;
  for (let i = 0; i < n; i++) {
    const p = m.get(`k${i}`);
    if (p !== undefined) {
      hits++;
      sum = (sum + p) | 0;
    }
  }
  // miss lookups — the "m" prefix is never inserted
  let misses = 0;
  for (let i = 0; i < n; i++) {
    if (m.get(`m${i}`) === undefined) misses++;
  }
  // remove every third key
  let removed = 0;
  for (let i = 0; i < n; i += 3) {
    if (m.delete(`k${i}`)) removed++;
  }
  // full re-scan: two thirds survive
  let present = 0;
  for (let i = 0; i < n; i++) {
    if (m.has(`k${i}`)) present++;
  }
  // re-add the removed keys under negated values
  let added2 = 0;
  for (let i = 0; i < n; i += 3) {
    const k = `k${i}`;
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

console.log("CHECKSUM " + churn(50000));
