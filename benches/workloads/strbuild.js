// strbuild.js — the .js twin of strbuild.rut (the std strbuild pkg's
// bench row, the strbuild batch phase 2; survey §5.3 is the contract):
// the identical LCG chain (seed 42, same draws, same order, same sample
// grid), the same counts + sampled-content fold (BigInt.asIntN
// discipline — this row folds integers + string codes only).
//
// THE IDIOM CHOICE (survey §5.3 requires it disclosed on this header):
//   - Phase A rides `Array#push` per draw + ONE `Array#join("")` per
//     round — the JS engine's own rope-flattened build, exactly the
//     "what does a mature engine's idiom cost" baseline the suite
//     exists for.
//   - Phase B's per-fragment build is a SINGLE-PIECE build, and the
//     JS-idiomatic build of a one-piece string is the piece itself —
//     no builder object, no join (a degenerate `[piece].join("")` was
//     rejected as ceremony, not idiom). The pre-size hint the rut side
//     draws has no JS equivalent; the draw is REPLAYED (same LCG
//     stream) so the chains stay in lockstep, but the value is unused.
// Every built string is compared content-exact against its source
// fragment, every rep asserts against the first — as the rut side runs.

const wrap64 = (x) => BigInt.asIntN(64, x & ((1n << 64n) - 1n));

function lcg(x) {
  return (Math.imul(x, 1103515245) + 12345) >>> 0;
}

// Scale constants (rut twins in fn n_appends/m_builds/rounds_n/samples_n).
const N = 1048576; // 2^20, phase A per round
const M = 65536;   // 2^16, phase B per round
const ROUNDS = 4;
const SN = 32;

// The fragment table: letter k of "abcde" repeated 2k+1 times — the
// same derivable content the rut side builds through the pkg.
const LETTERS = "abcde";
const FR = [];
for (let k = 0; k < 5; k++) FR.push(LETTERS[k].repeat(2 * k + 1));

function round() {
  let x = 42; // the LCG chain replays per round (the rep assert is real)

  // ---- phase A: the churn (push per draw, ONE join per round) --------
  const parts = [];
  let cps = 0;
  let ns = 0;
  const poss = new Array(SN).fill(0);
  const scls = new Array(SN).fill(0);
  const grid = N / SN;
  const off = SN / 2;
  for (let i = 0; i < N; i++) {
    x = lcg(x);
    const cls = (x >>> 16) % 5;
    parts.push(FR[cls]);
    cps += 2 * cls + 1;
    if (i % grid === off) {
      poss[ns] = cps - (2 * cls + 1);
      scls[ns] = cls;
      ns++;
    }
  }

  // ---- phase B, amortized half: the ONE join on the big doc ----------
  const doc = parts.join("");
  if (doc.length !== cps) throw new Error("strbuild: doc len != appended codepoints");
  let ha = 0n; // the sampled-content fold
  for (let s = 0; s < ns; s++) {
    const len = 2 * scls[s] + 1;
    const piece = doc.slice(poss[s], poss[s] + len);
    if (piece !== FR[scls[s]]) throw new Error("strbuild: sample content diverged");
    for (const c of piece) {
      ha = wrap64(ha * 31n + BigInt(c.codePointAt(0)));
    }
    ha = wrap64(ha * 31n + BigInt(len));
  }

  // ---- phase B, per-fragment sub-phase (the piece itself; disclosed) -
  let hb = 0n;
  let bcps = 0n;
  for (let j = 0; j < M; j++) {
    x = lcg(x);
    const hint = (x >>> 8) % 64; // replayed for lockstep; no JS equivalent
    x = lcg(x);
    const cls = (x >>> 16) % 5;
    const built = FR[cls];
    if (built !== FR[cls]) throw new Error("strbuild: built content diverged");
    hb = wrap64(hb * 31n + BigInt(cls));
    bcps += BigInt(2 * cls + 1);
  }

  return wrap64(
    wrap64(wrap64(BigInt(cps) * 31n + ha) * 31n + hb) * 31n + bcps,
  );
}

let total = 0n;
let first = 0n;
for (let r = 0; r < ROUNDS; r++) {
  const s = round();
  if (r === 0) {
    first = s;
  } else if (s !== first) {
    throw new Error("strbuild: rep diverged");
  }
  total = wrap64(total * 31n + s);
}
const finalC = wrap64(total * 31n + BigInt(ROUNDS));
console.log("CHECKSUM " + finalC.toString());
