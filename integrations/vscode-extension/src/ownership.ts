// The workspace scan's pure path math — project discovery's merge, a
// file's owning project, and the mod-path `src/` shell. No `vscode`
// here on purpose: test/workspace-units.js requires the compiled
// module under plain node and pins the multi-project cases (sibling
// manifests, a root manifest beside nested ones, the folder bound).

import * as path from 'node:path';

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
