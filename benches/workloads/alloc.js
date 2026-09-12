// alloc.js — must match alloc.rut.
let s = 0;
for (let i = 0; i < 2000000; i++) {
  const p = { x: i, y: (i + 1) | 0 };
  s = (s + p.x + p.y) | 0;
}
console.log("CHECKSUM " + s);
