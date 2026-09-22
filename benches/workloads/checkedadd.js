// checkedadd.js — must match checkedadd.rut. The value lane is
// ±2^63-scale (b sits at i64::MAX - 1000), so the accumulation runs in
// BigInt: exact i64 wrapping via BigInt.asIntN, identical bits to rut's.
const B = 9223372036854774807n; // i64::MAX - 1000
const MAX = 9223372036854775807n;
let s = 0n;
let bad = 0n;
for (let i = 0; i < 10000000; i++) {
  const a = BigInt((i % 2048) - 1024);
  const t = a + B; // the exact sum; overflow iff a > 1000
  const ok = t <= MAX;
  const v = BigInt.asIntN(64, t); // the wrapped value lane
  s = BigInt.asIntN(64, s + v);
  if (!ok) {
    bad = BigInt.asIntN(64, bad + 1n);
  }
}
s = BigInt.asIntN(64, s + bad);
console.log("CHECKSUM " + s.toString());
