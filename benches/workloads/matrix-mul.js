// matrix-mul.js — must match workloads/matrix-mul.rut.
const N = 64;

const a = new Float64Array(N * N);
const b = new Float64Array(N * N);
const out = new Float64Array(N * N);
for (let i = 0; i < N * N; i++) {
  a[i] = i;
  b[i] = i % 7;
}
for (let i = 0; i < N; i++) {
  for (let k = 0; k < N; k++) {
    const aik = a[i * N + k];
    for (let j = 0; j < N; j++) {
      out[i * N + j] += aik * b[k * N + j];
    }
  }
}
let s = 0;
for (let i = 0; i < N * N; i++) s += out[i];
console.log("CHECKSUM " + s);
