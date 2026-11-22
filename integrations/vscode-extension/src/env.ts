// The read-only HostEnv adapter — the capability the crate's dep walk
// and the mod-name completion slot's disk probe read through (the
// `HostEnv` contract in the crate's `js/index.d.ts`, the one
// declaration file). node:fs SYNC calls only (the walk is one fast
// handler turn inside the wasm — it cannot await); a miss is
// null/false, never a throw; `notify` rides the vscode message lane.
// The grant is read-only by construction: no write, no delete, no
// fetch exists here; the one addition is the CLOCK (`wakeAfter` —
// plain setTimeout, machinery only, never a business callback) — the
// extension writes NOTHING, anywhere. The env path law
// (`resolveHostPath` — the `.rut/` store anchor, the `file://`
// doc-uri resolution) lives in `ownership.ts` beside the other pure
// path math; every read below resolves through it.

import * as fs from 'node:fs';
import * as vscode from 'vscode';
import { resolveHostPath } from './ownership';
import type { HostEnv } from './wasm';

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
