// binary-trees.js — must match workloads/binary-trees.rut.
function make(depth) {
  if (depth <= 0) return { l: null, r: null };
  return { l: make(depth - 1), r: make(depth - 1) };
}

function count(n) {
  let c = 1;
  if (n.l !== null) c += count(n.l);
  if (n.r !== null) c += count(n.r);
  return c;
}

console.log("CHECKSUM " + count(make(14)));
