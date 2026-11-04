// the workspace index — the wasm face's fs scan (the CLI's install
// lane walks the same shape): every `**/*.rut` under the workspace folders indexes into the
// SAME `defs` store the dep walk fills, `mod.rut` files under a
// folder-level package stamped with their (pkg, mod path) pair.

import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { MANIFEST_NAME, MAX_WORKSPACE_FILES } from './constants';
import { hintOnce, readFileNullable } from './util';
import { isEnvelope, type RutLsp } from './wasm';

// the mod-tree child budget — this scan's own cap (the walk's caps
// live Rust-side, in the crate that owns the walk)
const MAX_MOD_FILES = 200;

// the folder-level package's name (its manifest parses in the wasm) —
// `null` when the folder has no manifest or a broken one
export async function folderPkgName(rut: RutLsp, folder: vscode.Uri): Promise<string | null> {
    const bytes = await readFileNullable(vscode.Uri.joinPath(folder, MANIFEST_NAME));
    if (bytes === null) {
        return null;
    }
    const table = await rut.parseManifest(new TextDecoder().decode(bytes));
    return isEnvelope(table) ? null : table.name;
}

// is there a `rut.jsonc` between the file's dir and the folder root
// (inclusive of the file's dir, exclusive of the folder)? Then the
// file belongs to a NESTED package — its own manifest walk owns its
// mod paths, the folder stamp would lie
function nestedManifest(folderFs: string, fileFs: string): boolean {
    let dir = path.dirname(fileFs);
    while (dir.length > folderFs.length) {
        if (fs.existsSync(path.join(dir, MANIFEST_NAME))) {
            return true;
        }
        const parent = path.dirname(dir);
        if (parent === dir) {
            break;
        }
        dir = parent;
    }
    return false;
}

// a package file's dir relative to the manifest, minus the leading
// `src` component, is its mod path (`src` → `''` the root,
// `src/layout` → `layout`) — the package's module tree lives under its
// `src/` shell. Anything outside the shell (the old
// root-beside-manifest spelling included) is not a module file: `null`.
function stripSrcShell(rel: string): string | null {
    const norm = rel.split(path.sep).join('/');
    if (norm === 'src') {
        return '';
    }
    if (norm.startsWith('src/')) {
        return norm.slice('src/'.length);
    }
    return null;
}

export async function indexWorkspace(rut: RutLsp): Promise<void> {
    const files = await vscode.workspace.findFiles(
        '**/*.rut',
        '{**/target/**,**/node_modules/**,**/.git/**,**/.*/**}',
        MAX_WORKSPACE_FILES,
    );
    const dec = new TextDecoder();
    const pkgs = new Map<string, string | null>();
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
        pkgs.set(folder.uri.fsPath, await folderPkgName(rut, folder.uri));
    }
    // the BFS frontier: stamped files whose `mod NAME;` edges may carry
    // single-file children (`NAME.rut` beside the declaring file)
    const frontier: Array<{
        folder: vscode.Uri;
        dir: vscode.Uri;
        pkg: string;
        modPath: string;
        decls: Array<{ name: string; vis: 'pub' | 'mod' }>;
    }> = [];
    for (const file of files) {
        const bytes = await vscode.workspace.fs.readFile(file);
        const folder = vscode.workspace.getWorkspaceFolder(file)?.uri;
        const pkg = folder ? (pkgs.get(folder.fsPath) ?? null) : null;
        // a `mod.rut` under a folder-level package stamps its mod path
        // (the dir relative to the folder, minus the `src/` shell —
        // `src/mod.rut` is the root `''`) — the position-path tier's data.
        // A file outside the shell keeps today's plain shape (the dep walk
        // re-stamps the deps it mounts anyway)
        if (
            pkg &&
            folder &&
            path.basename(file.fsPath) === 'mod.rut' &&
            !nestedManifest(folder.fsPath, file.fsPath)
        ) {
            const rel = path.relative(folder.fsPath, path.dirname(file.fsPath));
            const modPath = stripSrcShell(rel);
            if (modPath === null) {
                await rut.addDef(file.toString(), dec.decode(bytes));
                continue;
            }
            const stamped = await rut.addDefMod(file.toString(), pkg, modPath, dec.decode(bytes));
            if (!isEnvelope(stamped) && stamped.decls.length > 0) {
                frontier.push({
                    folder,
                    dir: vscode.Uri.file(path.dirname(file.fsPath)),
                    pkg,
                    modPath,
                    decls: stamped.decls,
                });
            }
            continue;
        }
        await rut.addDef(file.toString(), dec.decode(bytes));
    }
    // the declared children, breadth-first: each stamped edge probes
    // `NAME.rut` beside the declaring file (BOTH spellings is the
    // loader's loud ambiguity — hinted once). A child stamps at (pkg,
    // child path) and its own decls enqueue — declarations are the
    // graph, bounded by this scan's own file budget.
    let budget = MAX_MOD_FILES;
    while (frontier.length > 0 && budget > 0) {
        // the loop guard guarantees a node — shift() only answers
        // undefined on an empty frontier
        const next = frontier.shift();
        if (next === undefined) {
            break;
        }
        const { folder, dir, pkg, modPath, decls } = next;
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
            if (nestedManifest(folder.fsPath, childUri.fsPath)) {
                continue; // a nested package's own walk owns it
            }
            const childPath = modPath === '' ? decl.name : `${modPath}/${decl.name}`;
            if (!hasFile) {
                // the directory form is the path-shape stamp's business —
                // EXCEPT under a file-form parent, whose child's bare
                // relative dir (minus the `src/` shell) is not its mod path;
                // only that correction spends the budget here
                const bareRel = path.relative(folder.fsPath, path.dirname(childUri.fsPath));
                if (stripSrcShell(bareRel) === childPath) {
                    continue;
                }
            }
            const bytes = await vscode.workspace.fs.readFile(childUri);
            // addDefMod replaces per origin — a dir-form correction overrides
            // the path-shape stamp's bare-relative answer
            const stamped = await rut.addDefMod(
                childUri.toString(),
                pkg,
                childPath,
                dec.decode(bytes),
            );
            budget--;
            if (!isEnvelope(stamped) && stamped.decls.length > 0) {
                frontier.push({
                    folder,
                    dir: hasFile ? dir : vscode.Uri.joinPath(dir, decl.name),
                    pkg,
                    modPath: childPath,
                    decls: stamped.decls,
                });
            }
        }
    }
}
