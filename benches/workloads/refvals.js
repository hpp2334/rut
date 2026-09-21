// refvals.js — the JS twin of refvals/main.rut: the identical
// record-valued churn with JS objects as the values. This is the
// natural twin shape for the row: property mutation through a retrieved
// reference IS JS's own semantics — `p.x += 1` on the object a Map.get
// returned writes through to the stored value, exactly the one-cell
// aliasing law the rut side pins (RFC 0044). Same keys, same values,
// same counters; every term of the checksum is an exact integer below
// 2^53, so the doubles compute the rut side's number bit-exactly.
// Scale: n = 100000 entries on the main map, gn = 200000 on the grow
// sweep. Adaptation (the suite's standing one): JS Map has no
// insert-or-replace primitive, so the add/replace split costs one extra
// `has` probe per put compared with rut's `put -> bool`.

const GOLD = -1640531535; // 0x9E3779B1 read as i32

function key(i) {
  return (i * GOLD) | 0;
}

function churn(n, gn) {
  const m = new Map();

  // (1a) build — every key new; one fresh object per put
  let added = 0;
  for (let i = 0; i < n; i++) {
    const xi = i;
    if (!m.has(key(i))) added++;
    m.set(key(i), { x: xi, y: xi + 1 });
  }

  // (1b) overwrite churn — every key present, fresh object per put
  let replaced = 0;
  for (let i = 0; i < n; i++) {
    const xi = i;
    if (m.has(key(i))) replaced++;
    m.set(key(i), { x: xi + 7, y: xi + 8 });
  }

  // (2a) hit gets — both fields
  let sumBefore = 0;
  let hits = 0;
  for (let i = 0; i < n; i++) {
    const p = m.get(key(i));
    if (p !== undefined) {
      hits++;
      sumBefore += p.x + p.y;
    }
  }

  // (2b) miss gets — 5:1 hit-heavy overall
  let misses = 0;
  for (let i = 0; i < n / 5; i++) {
    if (m.get(key(2 * n + i)) === undefined) misses++;
  }

  // (3) read-modify-write through the alias — p IS the stored object
  let rmw = 0;
  for (let i = 0; i < n; i++) {
    const p = m.get(key(i));
    if (p !== undefined) {
      p.x += 1;
      rmw++;
    }
  }

  // (3b) read-back over fresh gets — observes every write-through
  let sumAfter = 0;
  let hits2 = 0;
  for (let i = 0; i < n; i++) {
    const p = m.get(key(i));
    if (p !== undefined) {
      hits2++;
      sumAfter += p.x;
    }
  }

  // (4) grow-heavy sweep — a fresh map through every load-factor
  // boundary, then an overwrite pass and a full read-back of y
  const g = new Map();
  let addedG = 0;
  for (let i = 0; i < gn; i++) {
    const xi = i;
    if (!g.has(key(i))) addedG++;
    g.set(key(i), { x: xi + 1, y: xi * 2 });
  }
  let replacedG = 0;
  for (let i = 0; i < gn; i++) {
    const xi = i;
    if (g.has(key(i))) replacedG++;
    g.set(key(i), { x: xi + 3, y: xi * 2 });
  }
  let hitsG = 0;
  let sumG = 0;
  for (let i = 0; i < gn; i++) {
    const p = g.get(key(i));
    if (p !== undefined) {
      hitsG++;
      sumG += p.y;
    }
  }

  // (5) removal churn on m
  let removed = 0;
  for (let i = 0; i < n; i += 2) {
    if (m.delete(key(i))) removed++;
  }
  let present = 0;
  for (let i = 0; i < n; i++) {
    if (m.has(key(i))) present++;
  }
  let added2 = 0;
  for (let i = 0; i < n; i += 2) {
    const xi = i;
    if (!m.has(key(i))) added2++;
    m.set(key(i), { x: xi * 3, y: xi });
  }

  // the checksum — identical weight table, exact in doubles
  let c = sumBefore;
  c += hits * 13;
  c += misses * 17;
  c += replaced * 11;
  c += added * 7;
  c += rmw * 19;
  c += sumAfter * 2;
  c += hits2 * 23;
  c += addedG * 29;
  c += replacedG * 31;
  c += hitsG * 37;
  c += sumG * 3;
  c += removed * 41;
  c += present * 43;
  c += added2 * 47;
  c += m.size * 53;
  c += g.size * 59;
  return c;
}

console.log("CHECKSUM " + churn(100000, 200000));
