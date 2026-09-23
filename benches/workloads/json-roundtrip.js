// json-roundtrip.js — the JS twin of json-roundtrip/main.rut (the std
// json pkg's row, rut-json batch phase 2, survey §2.11): the identical
// generated document (json-decode's genDoc — the knucleotide LCG, seed
// 42, same draws, same order), decoded by the engine's own `JSON.parse`
// and RE-ENCODED with `JSON.stringify` (the same job the rut side does:
// decode + encode through its lib's trait path), then the same value
// fold.
//
// The checksum is defined over the ROUND-TRIPPED VALUES, not lexemes —
// string codepoint folds (BigInt.asIntN discipline), i64 values, bools
// 0/1, and f64s through their SHORTEST-ROUND-TRIP DECIMAL (String(v) —
// the same algorithm as rut's f"{v}"; at this doc's magnitudes both
// print the identical plain decimal). The stringify result itself is
// kept live through a rep-stable length assert, mirroring the rut
// side's text.len() gate; it is not part of the checksum. Every rep
// asserts against the first.

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

// The value fold — the identical discipline main.rut runs over the
// decoded rows. Strings fold codepoints (for..of iterates code points,
// matching rut's str iteration); i64s fold their value; bools fold 0/1;
// f64s fold String(v) through the string fold (see the header).
function foldStr(h, s) {
  for (const ch of s) {
    h = wrap64(h * 31n + BigInt(ch.codePointAt(0)));
  }
  return h;
}

function rowSum(h, r) {
  h = wrap64(h * 31n + BigInt(r.id));
  h = foldStr(h, r.name);
  h = wrap64(h * 31n + (r.active ? 1n : 0n));
  h = foldStr(h, String(r.score));
  h = wrap64(h * 31n + BigInt(r.vals.length));
  for (const v of r.vals) h = wrap64(h * 31n + BigInt(v));
  h = wrap64(h * 31n + BigInt(r.tags.length));
  for (const t of r.tags) h = foldStr(h, t);
  h = r.note === null ? wrap64(h * 31n) : wrap64(h * 31n + BigInt(r.note));
  h = wrap64(h * 31n + BigInt(r.meta.rev));
  h = foldStr(h, r.meta.kind);
  return h;
}

function rowsSum(rows) {
  let t = BigInt(rows.length);
  for (const r of rows) t = rowSum(t, r);
  return t;
}

// Scale constants (rut twins in fn rows_n/vals_n/reps_n — identical to
// json-decode's).
const ROWS = 1200;
const VALS = 6;
const REPS = 3;

const doc = genDoc(ROWS, VALS);
let total = 0n;
let first = 0n;
let firstLen = 0;
for (let r = 0; r < REPS; r++) {
  const rows = JSON.parse(doc);
  const text = JSON.stringify(rows);
  if (r === 0) {
    firstLen = text.length;
  } else if (text.length !== firstLen) {
    throw new Error("json-roundtrip: encode length diverged");
  }
  const s = rowsSum(rows);
  if (r === 0) {
    first = s;
  } else if (s !== first) {
    throw new Error("json-roundtrip: rep diverged");
  }
  total = wrap64(total * 31n + s);
}
const finalC = wrap64(total * 31n + BigInt(REPS));
console.log("CHECKSUM " + finalC.toString());
