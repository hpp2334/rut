/* A tiny rut tokenizer for the code samples — enough to color
 * keywords, comments, strings (f-strings whole), numbers, and
 * Capitalized identifiers as types. Pure: line in, token list out. */

const RUT_KW = new Set((
  "entry fn let struct class interface enum impl use mod pub return self Self nil true false " +
  "if else for in while loop match when async static builtin"
).split(" "));

export function tokenizeRut(line) {
  const out = [];
  let i = 0;
  if (line.trim().startsWith("//")) return [{ t: "com", v: line }];
  while (i < line.length) {
    const c = line[i];
    if (/\s/.test(c)) {
      let j = i; while (j < line.length && /\s/.test(line[j])) j++;
      out.push({ t: "id", v: line.slice(i, j) }); i = j; continue;
    }
    if (c === "/" && line[i + 1] === "/") { out.push({ t: "com", v: line.slice(i) }); break; }
    if (c === '"') {
      let j = i + 1;
      while (j < line.length && line[j] !== '"') j += line[j] === "\\" ? 2 : 1;
      out.push({ t: "str", v: line.slice(i, j + 1) }); i = j + 1; continue;
    }
    if (/[0-9]/.test(c)) {
      let j = i; while (j < line.length && /[0-9._]/.test(line[j])) j++;
      out.push({ t: "num", v: line.slice(i, j) }); i = j; continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      let j = i; while (j < line.length && /[A-Za-z0-9_]/.test(line[j])) j++;
      const v = line.slice(i, j);
      if (v === "f" && line[j] === '"') {              /* f-string, whole thing */
        let k = j + 1; while (k < line.length && line[k] !== '"') k += line[k] === "\\" ? 2 : 1;
        out.push({ t: "str", v: line.slice(i, k + 1) }); i = k + 1; continue;
      }
      const t = RUT_KW.has(v) ? "kw" : (/^[A-Z]/.test(v) ? "typ" : "id");
      out.push({ t, v }); i = j; continue;
    }
    let j = i; while (j < line.length && !/[A-Za-z0-9_"\s]/.test(line[j])) j++;
    if (j === i) j++;
    out.push({ t: "pun", v: line.slice(i, j) }); i = j;
  }
  return out;
}

export const Tok = ({ tk }) => <span className={"tk-" + tk.t}>{tk.v}</span>;

/* split a token list at byte offset `at` (a hotspot dot lands there) */
export function splitTokens(toks, at) {
  let acc = 0, left = [], right = [];
  for (const tk of toks) {
    const start = acc, end = acc + tk.v.length;
    if (end <= at) left.push(tk);
    else if (start >= at) right.push(tk);
    else { left.push({ t: tk.t, v: tk.v.slice(0, at - start) });
           right.push({ t: tk.t, v: tk.v.slice(at - start) }); }
    acc = end;
  }
  return [left, right];
}
