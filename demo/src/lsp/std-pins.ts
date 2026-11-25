/**
 * The playground's std pin table — the SAME pins `rut new` scaffolds
 * (the std-v19 re-cut's `crates/rut-cli/template/rut.lock`),
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
export const PIN_SHA = "da9f33fcea953ab0f2feb1de5b6c5fa901ff804c";

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
  { name: "flow", url: stdZipUrl("flow"), sha256: "5987d7691269ea5fbf4a4cdf79733b9339a55ee57daa2d7f9b761acc3999c980" },
  { name: "futures", url: stdZipUrl("futures"), sha256: "c80977aa16abe61a7b7616703d688a1ae9100836f96806b70cd64e63aef7668e" },
  { name: "http", url: stdZipUrl("http"), sha256: "dab590862e8d7bc0fd2e236f501136c17c32730215eaf2f67d7dd01e19a61a82" },
  { name: "ink", url: stdZipUrl("ink"), sha256: "359a93a2e32f2ef09ae710b81b57ba0a691b588971dcf7110301b39a9ffba11e" },
  { name: "json", url: stdZipUrl("json"), sha256: "50ac914f50c4903a4c4806381c24ddc7c54370edb7189512906fe01aa3135e15" },
  { name: "nmapset", url: stdZipUrl("nmapset"), sha256: "959ae7cc249b4b5ccb953487a917edf96b282c29c5a53943c2d060c361421eb5" },
  { name: "pouch", url: stdZipUrl("pouch"), sha256: "ce5f554cdf6f6dbfe758281e598e18b46158db7be26277b662c9ed3c1a5c71a7" },
  { name: "strbuild", url: stdZipUrl("strbuild"), sha256: "b1000f1752fd24970c9bb0e519a45fbf7184d98a54318c0a19edccae4a4c273c" },
];
