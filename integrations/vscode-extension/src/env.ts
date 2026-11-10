// The read-only HostEnv adapter — the capability the crate's dep walk
// reads through (the `HostEnv` contract in the crate's `js/index.d.ts`,
// the one declaration file). node:fs SYNC calls only (the walk is one
// fast handler turn inside the wasm — it cannot await); a miss is
// null/false, never a throw; `notify` rides the vscode message lane.
// The grant is read-only by construction: no write, no delete, no
// fetch exists here; the one addition is the CLOCK (`wakeAfter` —
// plain setTimeout, machinery only, never a business callback) — the
// extension writes NOTHING, anywhere.

import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import * as vscode from 'vscode';
import type { HostEnv } from './wasm';

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
// that spelling back onto the real home.
export function resolveHostPath(p: string): string {
    if (p.startsWith('.rut/')) {
        return path.join(rutHome(), p.slice('.rut/'.length));
    }
    return path.normalize(p);
}

// the adapter itself — pure reads, the vscode hint lane, nothing else
export const hostEnv: HostEnv = {
    read(p) {
        try {
            return fs.readFileSync(resolveHostPath(p));
        } catch {
            return null; // a miss is a null, never a throw
        }
    },
    readDir(p) {
        try {
            return fs.readdirSync(resolveHostPath(p));
        } catch {
            return null; // no dir (or unreadable) — null
        }
    },
    isDir(p) {
        try {
            return fs.statSync(resolveHostPath(p)).isDirectory();
        } catch {
            return false; // no such path names a directory
        }
    },
    notify(message, isError) {
        const shown = isError
            ? vscode.window.showErrorMessage(message)
            : vscode.window.showInformationMessage(message);
        void Promise.resolve(shown).catch(() => undefined);
    },
    // the clock — the ONE machinery addition: the core's deferred work
    // (the dep walk's debounce window) arms through this. Plain
    // `setTimeout`, nothing more: the host supplies power, the ACTOR
    // owns every when — the wake posts an identity-free tick into the
    // core's own event loop, never a callback into host business.
    wakeAfter(ms, wake) {
        setTimeout(wake, ms);
    },
};
