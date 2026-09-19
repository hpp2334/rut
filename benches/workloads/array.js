// array.js — must match array.rut: a plain JS Array is the natural twin
// of rut's Vec<i32> (push through the grow path, indexed read+write,
// full iteration, same scale and op order).
const N = 1000000;
const M = 1000003;

function churn() {
  const v = [];
  for (let i = 0; i < N; i++) v.push(i % M);
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const x = v[i];
    v[i] = (x + 1) % M;
    acc += x;
  }
  let total = 0;
  for (const x of v) total += x;
  return acc + total;
}

console.log("CHECKSUM " + churn());
