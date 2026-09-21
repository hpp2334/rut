// nmap-hashset.js — the JS twin of nmap-hashset/main.rut (and the same
// algorithm as the removed hashset.js mapset twin): the identical
// HashSet<i32> churn on
// `new Set()`. The rut side swaps the removed pure-rut `mapset` for
// the native-key
// `nmapset` pkg; the JS side is unchanged — same work, same checksum.
// Scale: n = 100000 keys. Adaptation: JS Set.add
// does not report whether the element was new, so the add/dup split
// costs one extra `has` probe per add compared with rut's `put` -> bool.

const GOLD = -1640531535; // 0x9E3779B1 read as i32

function churn(n) {
  const s = new Set();
  // add — every key is new
  let added = 0;
  for (let i = 0; i < n; i++) {
    const k = (i * GOLD) | 0;
    if (!s.has(k)) added++;
    s.add(k);
  }
  // duplicate adds — the lower half again
  let dups = 0;
  for (let i = 0; i < n / 2; i++) {
    const k = (i * GOLD) | 0;
    if (s.has(k)) dups++;
    s.add(k);
  }
  // membership: the whole key space is present
  let present = 0;
  for (let i = 0; i < n; i++) {
    if (s.has((i * GOLD) | 0)) present++;
  }
  // membership: the second half of the space is absent
  let absent = 0;
  for (let i = 0; i < n; i++) {
    if (!s.has(((i + n) * GOLD) | 0)) absent++;
  }
  // remove the even keys
  let removed = 0;
  for (let i = 0; i < n; i += 2) {
    if (s.delete((i * GOLD) | 0)) removed++;
  }
  // a second set over the multiples of three, counted against the
  // survivors (the odd keys) of the first — intersection-style
  const s2 = new Set();
  let s2added = 0;
  for (let i = 0; i < n; i += 3) {
    const k = (i * GOLD) | 0;
    if (!s2.has(k)) s2added++;
    s2.add(k);
  }
  let inter = 0;
  for (let i = 0; i < n; i++) {
    if (s.has((i * GOLD) | 0) && s2.has((i * GOLD) | 0)) inter++;
  }
  let c = 0;
  c = (c + added * 31) | 0;
  c = (c + dups * 37) | 0;
  c = (c + present * 41) | 0;
  c = (c + absent * 43) | 0;
  c = (c + removed * 47) | 0;
  c = (c + s2added * 53) | 0;
  c = (c + inter * 59) | 0;
  c = (c + s.size * 61) | 0;
  return c;
}

console.log("CHECKSUM " + churn(100000));
