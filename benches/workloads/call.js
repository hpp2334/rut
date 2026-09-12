// call.js — must match call.rut.
function fib(n) {
  if (n < 2) return n;
  return fib(n - 1) + fib(n - 2);
}
console.log("CHECKSUM " + fib(28));
