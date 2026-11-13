/**
 * The playground's std pin table — the SAME pins `rut new` scaffolds
 * (the std-v17 re-cut's `crates/rut-cli/template/rut.lock`),
 * baked here because the browser never reads the repo: the shas are
 * the committed `dist/std/*.zip` bytes the lock vouches, and the urls
 * pin the LANDED COMMIT that carries them on jsDelivr's gh lane (an
 * immutable sha ref serves before `publish-std` cuts the tag —
 * stricter than the tag once it exists, and the bytes are identical).
 *
 * A std re-cut (the next `pack-std` + template re-pin) moves this
 * table with it: the shas here must stay the lock's, and the sha ref
 * must name a commit whose `dist/std/` mirror carries exactly those
 * bytes (the boot lane verifies — a drifted fetch refuses the dep,
 * never mounts quiet bytes).
 */

/** the landed commit whose `dist/std/` mirror the fetches pin */
export const PIN_SHA = "b0c7f0fb5bed46e192725abd00ed5ae0f35bf1d9";

/** the jsDelivr gh-lane url one std zip serves at (the sha-pinned ref) */
export function stdZipUrl(name: string): string {
  return `https://cdn.jsdelivr.net/gh/hpp2334/rut@${PIN_SHA}/dist/std/${name}.zip`;
}

/** one std row: the manifest/lock name, the pinned url, the zip's sha256 */
export interface StdPin {
  name: string;
  url: string;
  sha256: string;
}

/** the nine std source pkgs, in lock order (sorted by url) */
export const STD_PINS: StdPin[] = [
  { name: "calc", url: stdZipUrl("calc"), sha256: "794f823b8af9d1c1bcf4e1c30c51f1c405954b6ffc1be9cbf9b64739faa3fa24" },
  { name: "flow", url: stdZipUrl("flow"), sha256: "19545c61748bffa057cf6e5bea29bab728417fbcc6b84759216f7f3f68bbebc1" },
  { name: "futures", url: stdZipUrl("futures"), sha256: "e143211a0a6e9ddb0a3ccb5e3e79796d85b2ee627ee4c63f046dc583a55eb711" },
  { name: "http", url: stdZipUrl("http"), sha256: "9c6855cd58d7d888515fff691787a42708cef512423dc6040f8e214dbd00cbac" },
  { name: "ink", url: stdZipUrl("ink"), sha256: "eb2b568316efb77286cc5d94bec24a94f45d528cf244f54b91cb0d619dc53cc1" },
  { name: "json", url: stdZipUrl("json"), sha256: "653867a4451d8eb991cfdd59c9196d95c92ecc7b3da530519cba15e68951b18a" },
  { name: "nmapset", url: stdZipUrl("nmapset"), sha256: "1a59e0e4cab803dcd176ac919160cb9dbbc305011fb563dcc1d8bcd82307f5dd" },
  { name: "pouch", url: stdZipUrl("pouch"), sha256: "5740cb0b874a2c9a81f1e0d9534520fe4067e5ec48320b82cf7147e2d6b34d36" },
  { name: "strbuild", url: stdZipUrl("strbuild"), sha256: "1eedb374bcb52419b9ec82d9ea6f23fb3534faa0a4c75e4d03afedc6d7dabe33" },
];
