// mathhost.js — must match mathhost.rut.
let acc = 0.0;
let x = 1.5;
let y = 0.25;
for (let i = 0; i < 1000000; i++) {
  x = x * 0.9999998 + 0.0000003;
  y = y * 0.9999999 + 0.0000002;
  if (x < 0.25) { x = x + 2.75; }
  if (y < 0.10) { y = y + 1.50; }
  acc = acc + Math.abs(x - y);
  acc = acc + Math.min(x, y);
  acc = acc + Math.max(x, y);
  acc = acc + Math.sign(x - y) * 0.5;
}
console.log("CHECKSUM " + acc);
