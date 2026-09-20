// crossing-nop.js — must match crossing-nop.rut: the same four loops
// (host nop / inline twin / 4-param pair) over plain JS fn calls. JS
// has no host boundary, so both loops of each pair measure the same
// plain-call work — the row places rut's crossing against the field's
// plain call cost, and the checksum pins the arithmetic.
// Scale: 2_000_000 iterations per loop, 4 loops.
function nop(x) {
  return x;
}
function nopRut(x) {
  return x;
}
function nop4(a, b, c, d) {
  return a + b + c + d;
}
function nop4Rut(a, b, c, d) {
  return a + b + c + d;
}
let acc = 0;
// A: the "host" nop
for (let i = 0; i < 2000000; i++) {
  acc = acc + nop(i);
}
// B: the inline twin
for (let i = 0; i < 2000000; i++) {
  acc = acc + nopRut(i);
}
// A4: the 4-param nop
for (let i = 0; i < 2000000; i++) {
  acc = acc + nop4(i, i + 1, i + 2, i + 3);
}
// B4: the 4-param inline twin
for (let i = 0; i < 2000000; i++) {
  acc = acc + nop4Rut(i, i + 1, i + 2, i + 3);
}
console.log("CHECKSUM " + acc);
