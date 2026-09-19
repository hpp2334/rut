// json-decode.js — the JS twin of json-decode/main.rut: the identical
// generated document, decoded by the engine's own `JSON.parse` (this
// workload's JS-side is native-JSON-shaped, the way a qjs/node program
// does actual JSON work — the rut side chews the same text through the
// digest decoder), then the same downcast fold over the parsed value.
//
// The document generator is the knucleotide LCG (seed 42) run in the
// identical draw order; every number/string/bool lands as the same
// JSON text both sides. The fold wraps i64 (BigInt.asIntN) and sums
// codepoints — mirroring main.rut bit-for-bit; every rep asserts
// against the first.
//
// Scale: ROWS x VALS values per row, REPS reps (the pre-pooling,
// pre-inline baseline for the native-fastpath batch).

const MASK64 = (1n << 64n) - 1n;
const wrap64 = (x) => BigInt.asIntN(64, x & MASK64);

function lcg(x) {
  return (Math.imul(x, 1103515245) + 12345) >>> 0;
}

const ALPHA = "abcdefghijklmnopqrstuvwx";

const b2s = (b) => (b ? "true" : "false");

// sign+magnitude draw -> JSON number text (never "-0"; the rut twin
// emits the same digits for the same draw)
function valText(x) {
  const sign = (x >>> 13) % 2;
  const mag = (x >>> 19) % 100000;
  if (sign === 1 && mag !== 0) return `-${mag}`;
  return `${mag}`;
}

// Each record draws in the fixed order: id, name, active, score, six
// vals, tags, meta (rev + kind) — exactly main.rut's gen_doc.
function genDoc(rows, valsN) {
  const parts = [];
  let x = 42;
  for (let i = 0; i < rows; i++) {
    if (i > 0) parts.push(",");
    x = lcg(x);
    const id = ((x >>> 5) % 899998) + 100000;
    x = lcg(x);
    const na = (x >>> 3) % 24;
    const nb = (x >>> 9) % 24;
    x = lcg(x);
    const ok = ((x >>> 7) % 2) === 0;
    x = lcg(x);
    const si = ((x >>> 11) % 9000) + 1000;
    const sf = ((x >>> 17) % 9000) + 1000;
    const vals = [];
    for (let k = 0; k < valsN; k++) {
      x = lcg(x);
      vals.push(valText(x));
    }
    x = lcg(x);
    const ta1 = (x >>> 3) % 24;
    const tb1 = (x >>> 9) % 24;
    const ta2 = (x >>> 13) % 24;
    const tb2 = (x >>> 21) % 24;
    const ta3 = (x >>> 25) % 24;
    const tb3 = (x >>> 1) % 24;
    x = lcg(x);
    const rev = ((x >>> 7) % 9000) + 1000;
    const ka = (x >>> 3) % 24;
    const kb = (x >>> 9) % 24;
    const row = `{"id":${id},"name":"mm-${ALPHA[na]}${ALPHA[nb]}","active":${b2s(ok)},"score":${si}.${sf},"vals":[${vals.join(",")}],"tags":["${ALPHA[ta1]}${ALPHA[tb1]}","${ALPHA[ta2]}${ALPHA[tb2]}","${ALPHA[ta3]}${ALPHA[tb3]}"],"note":null,"meta":{"rev":${rev},"kind":"${ALPHA[ka]}${ALPHA[kb]}"}}`;
    parts.push(row);
  }
  return `[` + parts.join("") + `]`;
}

// Downcast fold over the parsed value: string/key codepoint sums,
// numbers via BigInt(Math.trunc(v)), fixed constants for the scalar
// tags — the identical fold main.rut runs over the opaque tree.
function foldStr(h, s) {
  for (const ch of s) {
    h = wrap64(h * 31n + BigInt(ch.codePointAt(0)));
  }
  return h;
}

function jdocSum(v) {
  if (v === null) return 1n;
  if (typeof v === "boolean") return v ? 3n : 2n;
  if (typeof v === "number") return BigInt(Math.trunc(v));
  if (typeof v === "string") return foldStr(4n, v);
  if (Array.isArray(v)) {
    let t = 5n;
    for (const e of v) t = wrap64(t * 11n + jdocSum(e));
    return t;
  }
  let t = 6n;
  for (const k of Object.keys(v)) {
    t = foldStr(t, k);
    t = wrap64(t * 11n + jdocSum(v[k]));
  }
  return t;
}

// Scale constants (rut twins in fn rows_n/vals_n/reps_n).
const ROWS = 1200;
const VALS = 6;
const REPS = 3;

const doc = genDoc(ROWS, VALS);
let total = 0n;
let first = 0n;
for (let r = 0; r < REPS; r++) {
  const tree = JSON.parse(doc);
  const s = jdocSum(tree);
  if (r === 0) {
    first = s;
  } else if (s !== first) {
    throw new Error("json-decode: rep diverged");
  }
  total = wrap64(total * 31n + s);
}
const finalC = wrap64(total * 31n + BigInt(REPS));
console.log("CHECKSUM " + finalC.toString());
