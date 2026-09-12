// spectral-norm.js — must match workloads/spectral-norm.rut.
function fsqrt(x) {
  if (x <= 0) return 0;
  let r = x;
  for (let i = 0; i < 30; i++) r = 0.5 * (r + x / r);
  return r;
}

function a(i, j) {
  return 1 / ((i + j) * (i + j + 1) / 2 + i + 1);
}

function mulAv(v, out, n) {
  for (let i = 0; i < n; i++) {
    let s = 0;
    for (let j = 0; j < n; j++) s += a(i, j) * v[j];
    out[i] = s;
  }
}

function mulAtv(v, out, n) {
  for (let i = 0; i < n; i++) {
    let s = 0;
    for (let j = 0; j < n; j++) s += a(j, i) * v[j];
    out[i] = s;
  }
}

const n = 150;
const u = new Float64Array(n);
const v = new Float64Array(n);
const tmp = new Float64Array(n);
for (let i = 0; i < n; i++) u[i] = 1;
for (let it = 0; it < 10; it++) {
  mulAv(u, tmp, n);
  mulAtv(tmp, v, n);
  mulAv(v, tmp, n);
  mulAtv(tmp, u, n);
}
let vbv = 0;
let vv = 0;
for (let i = 0; i < n; i++) {
  vbv += u[i] * v[i];
  vv += v[i] * v[i];
}
console.log("CHECKSUM " + fsqrt(vbv / vv));
