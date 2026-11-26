// the workspace index — the wasm face's fs scan (the CLI's install
// lane walks the same shape): every `**/*.rut` under the workspace
// folders indexes into the SAME `defs` store the dep walk fills, each
// file stamped by its OWNING project — the nearest enclosing
// `rut.jsonc` dir — so one folder can carry several nested projects
// (the folder root need not be a project at all).

import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { IGNORED, MANIFEST_NAME, MAX_WORKSPACE_FILES } from './constants';
import { mergeProjectDirs, ownerDir, stripSrcShell } from './ownership';
import { hintOnce, readFileNullable } from './util';
import { isEnvelope, type RutLsp } from './wasm';

// the mod-tree child budget — this scan's own cap (the walk's caps
// live Rust-side, in the crate that owns the walk)
const MAX_MOD_FILES = 200;

// one discovered rut project: its manifest dir + the pkg name the
// manifest parses to (an unnamed manifest rides the dir's basename —
// the walk's own derivation; a broken one is no project at all)
export interface ProjectDir {
    dir: vscode.Uri;
    pkg: string;
}

// the parsed pkg names, memoized per manifest dir — discovery and the
// ownership walk re-ask the same dirs on every re-index. Invalidated
// wholesale (`invalidateProjects`) on the manifest watchers and the
// folder-change hook: a manifest landing, changing, or deleting
// re-shapes the project set.
const pkgCache = new Map<string, string | null>();

export function invalidateProjects(): void {
    pkgCache.clear();
}

// the dir's pkg name — `null` when there is no manifest or a broken
// one (memoized; the parse rides the wasm)
async function pkgOf(rut: RutLsp, dir: vscode.Uri): Promise<string | null> {
    const cached = pkgCache.get(dir.fsPath);
    if (cached !== undefined) {
        return cached;
    }
    const bytes = await readFileNullable(vscode.Uri.joinPath(dir, MANIFEST_NAME));
    let pkg: string | null = null;
    if (bytes !== null) {
        const table = await rut.parseManifest(new TextDecoder().decode(bytes));
        if (!isEnvelope(table)) {
            pkg = table.name ?? path.basename(dir.fsPath);
        }
    }
    pkgCache.set(dir.fsPath, pkg);
    return pkg;
}

// every rut project dir the editor can see: each workspace folder
// whose root carries a manifest, plus every `**/rut.jsonc` the scan
// finds (the shared ignore set — target/, node_modules/, .git/,
// dot-dirs are never projects). Sibling projects never shadow each
// other: each entry owns its own tree, at any nesting depth.
export async function discoverProjects(rut: RutLsp): Promise<ProjectDir[]> {
    const found = await vscode.workspace.findFiles(
        `**/${MANIFEST_NAME}`,
        IGNORED,
        MAX_WORKSPACE_FILES,
    );
    const out: ProjectDir[] = [];
    for (const fsPath of mergeProjectDirs(
        (vscode.workspace.workspaceFolders ?? []).map((f) => f.uri.fsPath),
        found.map((f) => path.dirname(f.fsPath)),
    )) {
        const dir = vscode.Uri.file(fsPath);
        const pkg = await pkgOf(rut, dir);
        if (pkg !== null) {
            out.push({ dir, pkg });
        }
    }
    return out;
}

export async function indexWorkspace(rut: RutLsp): Promise<void> {
    const files = await vscode.workspace.findFiles('**/*.rut', IGNORED, MAX_WORKSPACE_FILES);
    const dec = new TextDecoder();
    const owners = new Map((await discoverProjects(rut)).map((p) => [p.dir.fsPath, p.pkg]));
    const projectDirs = new Set(owners.keys());
    // the BFS frontier: stamped files whose `mod NAME;` edges may carry
    // single-file children (`NAME.rut` beside the declaring file)
    const frontier: Array<{
        owner: vscode.Uri;
        dir: vscode.Uri;
        pkg: string;
        modPath: string;
        decls: Array<{ name: string; vis: 'pub' | 'mod' }>;
    }> = [];
    for (const file of files) {
        const bytes = await vscode.workspace.fs.readFile(file);
        const folder = vscode.workspace.getWorkspaceFolder(file)?.uri;
        // the file's OWNING project stamps it: a `mod.rut` under the
        // owner's `src/` shell stamps its mod path (the dir relative to
        // the OWNER, minus the `src/` shell — `src/mod.rut` is the root
        // `''`) — the position-path tier's data. A file with no owner
        // stays the plain unstamped shape (the dep walk re-stamps the
        // deps it mounts anyway); an owned file outside the shell keeps
        // its package name but no mod path.
        const ownerFs = folder ? ownerDir(folder.fsPath, file.fsPath, projectDirs) : null;
        const pkg = ownerFs ? (owners.get(ownerFs) ?? null) : null;
        if (pkg && ownerFs && path.basename(file.fsPath) === 'mod.rut') {
            const rel = path.relative(ownerFs, path.dirname(file.fsPath));
            const modPath = stripSrcShell(rel);
            if (modPath === null) {
                await rut.indexFile(file.toString(), dec.decode(bytes), pkg);
                continue;
            }
            const stamped = await rut.indexModAt(file.toString(), pkg, modPath, dec.decode(bytes));
            if (!isEnvelope(stamped) && stamped.decls.length > 0) {
                frontier.push({
                    owner: vscode.Uri.file(ownerFs),
                    dir: vscode.Uri.file(path.dirname(file.fsPath)),
                    pkg,
                    modPath,
                    decls: stamped.decls,
                });
            }
            continue;
        }
        // the file's own package name rides the push whenever a manifest
        // names the folder; a file no manifest owns stays unnamed (the
        // core's uri-shape fallback)
        await rut.indexFile(file.toString(), dec.decode(bytes), pkg ?? undefined);
    }
    // the declared children, breadth-first: each stamped edge probes
    // `NAME.rut` beside the declaring file (BOTH spellings is the
    // loader's loud ambiguity — hinted once). A child stamps at (pkg,
    // child path) under the SAME owner and its own decls enqueue —
    // declarations are the graph, bounded by this scan's own file
    // budget.
    let budget = MAX_MOD_FILES;
    while (frontier.length > 0 && budget > 0) {
        // the loop guard guarantees a node — shift() only answers
        // undefined on an empty frontier
        const next = frontier.shift();
        if (next === undefined) {
            break;
        }
        const { owner, dir, pkg, modPath, decls } = next;
        for (const decl of decls) {
            if (budget <= 0) {
                break;
            }
            const dirChild = vscode.Uri.joinPath(dir, decl.name, 'mod.rut');
            const fileChild = vscode.Uri.joinPath(dir, `${decl.name}.rut`);
            const hasFile = fs.existsSync(fileChild.fsPath);
            const hasDir = fs.existsSync(dirChild.fsPath);
            if (hasFile && hasDir) {
                hintOnce(
                    `mods:${fileChild.fsPath}`,
                    `rut: ${pkg} — both ${decl.name}/mod.rut and ${decl.name}.rut exist; keep one`,
                    true,
                );
                continue;
            }
            if (!hasFile && !hasDir) {
                continue; // nothing under either spelling — nothing to stamp here
            }
            const childUri = hasFile ? fileChild : dirChild;
            if (ownerDir(owner.fsPath, childUri.fsPath, projectDirs) !== owner.fsPath) {
                continue; // a nested package's own manifest owns it
            }
            const childPath = modPath === '' ? decl.name : `${modPath}/${decl.name}`;
            if (!hasFile) {
                // the directory form is the path-shape stamp's business —
                // EXCEPT under a file-form parent, whose child's bare
                // relative dir (minus the `src/` shell) is not its mod path;
                // only that correction spends the budget here
                const bareRel = path.relative(owner.fsPath, path.dirname(childUri.fsPath));
                if (stripSrcShell(bareRel) === childPath) {
                    continue;
                }
            }
            const bytes = await vscode.workspace.fs.readFile(childUri);
            // indexModAt replaces per origin — a dir-form correction overrides
            // the path-shape stamp's bare-relative answer
            const stamped = await rut.indexModAt(
                childUri.toString(),
                pkg,
                childPath,
                dec.decode(bytes),
            );
            budget--;
            if (!isEnvelope(stamped) && stamped.decls.length > 0) {
                frontier.push({
                    owner,
                    dir: hasFile ? dir : vscode.Uri.joinPath(dir, decl.name),
                    pkg,
                    modPath: childPath,
                    decls: stamped.decls,
                });
            }
        }
    }
}
