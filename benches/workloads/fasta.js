// fasta.js — must match workloads/fasta.rut.
function fasta(n) {
  const parts = [];
  const letters = "ACGT";
  let x = 42;
  let checksum = 0;
  for (let i = 0; i < n; i++) {
    x = (Math.imul(x, 1103515245) + 12345) >>> 0;
    const idx = (x >>> 16) % 4;
    parts.push(letters[idx]);
    checksum += idx;
  }
  const out = parts.join("");
  return checksum + ":" + out.length;
}

console.log("CHECKSUM " + fasta(10000));
