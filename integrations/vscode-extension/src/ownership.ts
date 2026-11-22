// The workspace scan's pure path math — project discovery's merge, a
// file's owning project, the mod-path `src/` shell, and the env's one
// path law (`resolveHostPath`: the crate's `/`-joined rooted strings —
// the `.rut/` store anchor, the `file://` doc-uri resolution). No
// `vscode` here on purpose: test/workspace-units.js requires the
// compiled module under plain node and pins the multi-project cases
// (sibling manifests, a root manifest beside nested ones, the folder
// bound) and the path law. `src/env.ts` reads every crate-spelled
// path through `resolveHostPath` — the env path law lives HERE, with
// the rest of the pure path math.

import * as os from 'node:os';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';

// the real global rut home — `$RUT_HOME`, else `~/.rut` (the same
// derivation rut-driver spells natively). Read per call, so a host
// (or test) that swaps the env is answered with the world it now sees.
function rutHome(): string {
    return process.env.RUT_HOME ?? path.join(os.homedir(), '.rut');
}

// one crate-spelled path → a real on-disk path. The crate joins with
// `/` and never normalizes (`std::path` never enters), so `..` rows
// collapse HERE; and the wasm sandbox cannot read env vars, so its
// `rut_home()` degenerates to `.rut` and every global-store path it
// spells arrives RELATIVE (`.rut/deps/<name>@<pin>/…`) — the host maps
// that spelling back onto the real home. A `file://` input (the
// mod-name completion probe hands the open document's directory
// verbatim — the uri minus its last segment) resolves through
// `fileURLToPath`: percent-decoding and Windows drive letters for
// free; on a throw (a host-shaped pseudo-uri) it falls through to the
// plain-path arm.
export function resolveHostPath(p: string): string {
    if (p.startsWith('.rut/')) {
        return path.join(rutHome(), p.slice('.rut/'.length));
    }
    if (p.startsWith('file://')) {
        try {
            return fileURLToPath(p);
        } catch {
            // fall through — normalize the spelling we were handed
        }
    }
    return path.normalize(p);
}

// the discovery merge: the workspace folder roots first, then every
// found manifest's dir, deduped in first-seen order (a root manifest
// and a scan hit for the same dir are ONE project; sibling projects
// never shadow each other — only a same-dir hit dedupes)
export function mergeProjectDirs(folderFs: string[], foundFs: string[]): string[] {
    const dirs: string[] = [];
    const seen = new Set<string>();
    for (const dir of [...folderFs, ...foundFs]) {
        if (seen.has(dir)) {
            continue;
        }
        seen.add(dir);
        dirs.push(dir);
    }
    return dirs;
}

// a file's OWNING project dir — the nearest ancestor listed in
// `projects` (the discovered manifest dirs), walking from the file's
// own dir up to the workspace folder root INCLUSIVE (the root counts
// as an owner only when IT carries a manifest). `null` = no owner:
// the file stays a plain unstamped index entry. A manifest ABOVE the
// folder never owns — the folder bounds the walk.
export function ownerDir(folderFs: string, fileFs: string, projects: Set<string>): string | null {
    let dir = path.dirname(fileFs);
    for (;;) {
        if (projects.has(dir)) {
            return dir;
        }
        if (dir === folderFs) {
            return null; // the folder root itself has no manifest
        }
        const parent = path.dirname(dir);
        if (parent === dir) {
            return null; // past the fs root — a file no folder claims
        }
        dir = parent;
    }
}

// a package file's dir relative to the manifest, minus the leading
// `src` component, is its mod path (`src` → `''` the root,
// `src/layout` → `layout`) — the package's module tree lives under its
// `src/` shell. Anything outside the shell (the old
// root-beside-manifest spelling included) is not a module file: `null`.
export function stripSrcShell(rel: string): string | null {
    const norm = rel.split(path.sep).join('/');
    if (norm === 'src') {
        return '';
    }
    if (norm.startsWith('src/')) {
        return norm.slice('src/'.length);
    }
    return null;
}
