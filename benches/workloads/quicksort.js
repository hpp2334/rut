// quicksort.js — must match workloads/quicksort.rut.
const N = 10000;

function quicksort(xs, lo, hi) {
  if (lo >= hi) return;
  const pivot = xs[hi];
  let i = lo - 1;
  for (let j = lo; j < hi; j++) {
    if (xs[j] <= pivot) {
      i++;
      const t = xs[i];
      xs[i] = xs[j];
      xs[j] = t;
    }
  }
  const t = xs[i + 1];
  xs[i + 1] = xs[hi];
  xs[hi] = t;
  const p = i + 1;
  quicksort(xs, lo, p - 1);
  quicksort(xs, p + 1, hi);
}

const xs = new Int32Array(N);
let x = 42;
for (let i = 0; i < N; i++) {
  x = (Math.imul(x, 1664525) + 1013904223) | 0;
  xs[i] = x % 1000;
}
quicksort(xs, 0, N - 1);
let h = 0;
for (let i = 0; i < N; i++) h = (Math.imul(h, 31) + xs[i]) | 0;
console.log("CHECKSUM " + h);
