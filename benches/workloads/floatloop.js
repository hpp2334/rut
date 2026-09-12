// floatloop.js — must match floatloop.rut.
let s = 0;
for (let i = 0; i < 2000000; i++) {
  s = s * 1.0000001 + 0.5;
}
console.log("CHECKSUM " + s);
