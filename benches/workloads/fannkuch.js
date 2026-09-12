// fannkuch.js — must match workloads/fannkuch.rut.
const N = 7;

function fannkuch(n) {
  const perm = new Int32Array(n);
  const perm1 = new Int32Array(n);
  const count = new Int32Array(n);
  for (let i = 0; i < n; i++) perm1[i] = i;
  let maxFlips = 0;
  let checksum = 0;
  let permCount = 0;
  let r = n;
  let done = false;
  while (!done) {
    while (r !== 1) {
      count[r - 1] = r;
      r--;
    }
    for (let i = 0; i < n; i++) perm[i] = perm1[i];
    let flips = 0;
    while (perm[0] !== 0) {
      const k = perm[0] + 1;
      for (let i = 0, j = k - 1; i < j; i++, j--) {
        const t = perm[i];
        perm[i] = perm[j];
        perm[j] = t;
      }
      flips++;
    }
    if (flips > maxFlips) maxFlips = flips;
    if (perm1[0] !== 0 && perm1[n - 1] !== n - 1) {
      checksum += permCount % 2 === 0 ? flips : -flips;
    }
    while (r !== n) {
      const first = perm1[0];
      for (let i = 0; i < r; i++) perm1[i] = perm1[i + 1];
      perm1[r] = first;
      count[r]--;
      if (count[r] > 0) break;
      r++;
    }
    permCount++;
    if (r === n) done = true;
  }
  return checksum * 1000 + maxFlips;
}

console.log("CHECKSUM " + fannkuch(N));
