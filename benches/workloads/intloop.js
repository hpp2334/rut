// intloop.js — must match intloop.rut.
let s = 0;
for (let i = 0; i < 5000000; i++) {
  s = (s + Math.imul(i, 3) - 1) | 0;
}
console.log("CHECKSUM " + s);
