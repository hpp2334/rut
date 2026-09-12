// sieve.js — must match workloads/sieve.rut.
const LIMIT = 500000;

function sieve(limit) {
  const marks = new Uint8Array(limit + 1);
  const primes = [];
  for (let i = 2; i <= limit; i++) {
    if (marks[i] === 0) {
      primes.push(i);
      for (let m = i * i; m <= limit; m += i) marks[m] = 1;
    }
  }
  return primes;
}

console.log("CHECKSUM " + sieve(LIMIT).length);
