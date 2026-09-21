// nmap-knucleotide.js — the JS twin of nmap-knucleotide/main.rut (and
// the same algorithm as the removed knucleotide.js mapset twin): the
// identical k-mer counting
// on `new Map()` with string keys. The rut side swaps the removed
// pure-rut `mapset` for the
// native-key `nmapset` pkg; the JS side is unchanged — same work, same
// checksum. The sequence is generated with fasta's LCG (the harness
// takes no stdin — the same adaptation both sides). Scale: seq =
// 200000 letters.

function lcgSeq(n) {
  const letters = "ACGT";
  let x = 42;
  let seq = "";
  for (let i = 0; i < n; i++) {
    x = (Math.imul(x, 1103515245) + 12345) >>> 0;
    const idx = (x >>> 16) % 4;
    seq += letters[idx];
  }
  return seq;
}

// get-or-insert one 12-mer count, then 100 generated fragment probes —
// the same two-shapes-of-churn as count12 in main.rut.
function count12(m, seq, n) {
  for (let i = 0; i < n - 11; i++) {
    const kmer = seq.slice(i, i + 12);
    const p = m.get(kmer);
    if (p !== undefined) {
      m.set(kmer, p + 1);
    } else {
      m.set(kmer, 1);
    }
  }
  let fragSum = 0;
  for (let j = 0; j < 100; j++) {
    const pos = (j * 2497) % (n - 12);
    const p = m.get(seq.slice(pos, pos + 12));
    if (p !== undefined) fragSum = (fragSum + p) | 0;
  }
  return fragSum;
}

function mainChecksum(n) {
  const seq = lcgSeq(n);
  const m = new Map();
  const fragSum = count12(m, seq, n);
  // 1-mers: four keys, total n
  const m1 = new Map();
  for (let i = 0; i < n; i++) {
    const kmer = seq.slice(i, i + 1);
    const p = m1.get(kmer);
    if (p !== undefined) m1.set(kmer, p + 1);
    else m1.set(kmer, 1);
  }
  // 2-mers: sixteen keys, total n - 1
  const m2 = new Map();
  for (let i = 0; i < n - 1; i++) {
    const kmer = seq.slice(i, i + 2);
    const p = m2.get(kmer);
    if (p !== undefined) m2.set(kmer, p + 1);
    else m2.set(kmer, 1);
  }
  const letters = "ACGT";
  let sum1 = 0;
  let sum2 = 0;
  for (let a = 0; a < 4; a++) {
    const ca = letters.slice(a, a + 1);
    const p = m1.get(ca);
    if (p !== undefined) sum1 += p;
    for (let b = 0; b < 4; b++) {
      const cb = letters.slice(b, b + 1);
      const q = m2.get(ca + cb);
      if (q !== undefined) sum2 += q;
    }
  }
  let c = m.size;
  c = (c + fragSum) | 0;
  c = (c + sum1 * 2) | 0;
  c = (c + sum2 * 3) | 0;
  c = (c + (n - 11) * 5) | 0;
  return c;
}

console.log("CHECKSUM " + mainChecksum(200000));
