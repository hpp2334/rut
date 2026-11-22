// In-Extension-Host test — run via:
//   Code.exe --extensionDevelopmentPath=<ext> --extensionTestsPath=<this file>
// Exports run(); VS Code calls it after activation, exit code reflects the result.

const assert = require('node:assert');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const { join } = require('node:path');

const EXT_ID = 'rut.rut-vscode';
// the symbol fixture the test asserts on (relative to the extension, so
// the test runs on every machine) — the fixture's own module dir, its
// entry source is what the symbol assertions read
const FIXTURE = join(__dirname, 'fixtures', 'symbols', 'src', 'mod.rut');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function poll(what, fn, tries = 100, gap = 100) {
    for (let i = 0; i < tries; i++) {
        const v = await fn();
        if (v) return v;
        await sleep(gap);
    }
    throw new Error(`timed out waiting for ${what}`);
}

async function run() {
    const vscode = require('vscode');
    const log = (...a) => console.log('[rut-test]', ...a);

    // 1. the extension activates (opening a .rut doc fires onLanguage:rut)
    const uri = vscode.Uri.file(FIXTURE);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);
    const ext = vscode.extensions.getExtension(EXT_ID);
    assert.ok(ext, `extension ${EXT_ID} not found — check package.json name/publisher`);
    await poll('extension activation', () => ext.isActive);
    log('extension active');

    // 2. language registration
    assert.strictEqual(doc.languageId, 'rut', `languageId is ${doc.languageId}, not 'rut'`);
    log('languageId = rut');

    // 3. semantic tokens arrive (proves the client launched rut-lsp and it answered)
    const legend = await vscode.commands.executeCommand(
        'vscode.provideDocumentSemanticTokensLegend',
        doc.uri,
    );
    assert.ok(
        legend && Array.isArray(legend.tokenTypes) && legend.tokenTypes.includes('enumMember'),
        `bad legend: ${JSON.stringify(legend)}`,
    );
    const tokens = await poll('semantic tokens', () =>
        vscode.commands.executeCommand('vscode.provideDocumentSemanticTokens', doc.uri),
    );
    assert.ok(
        tokens?.data && tokens.data.length >= 100,
        `thin token stream: ${tokens?.data?.length}`,
    );
    log(
        `semantic tokens: ${tokens.data.length / 5} tokens, legend ok (${legend.tokenTypes.length} types)`,
    );

    // 4. document symbols
    const syms = await poll('document symbols', () =>
        vscode.commands.executeCommand('vscode.executeDocumentSymbolProvider', doc.uri),
    );
    assert.ok(Array.isArray(syms) && syms.length > 0, `no symbols: ${JSON.stringify(syms)}`);
    const names = syms.map((s) => s.name);
    for (const want of ['Color', 'Point', 'Circle', 'Drawable', 'main']) {
        assert.ok(names.includes(want), `symbol ${want} missing (got ${names.join(', ')})`);
    }
    log(`symbols: ${names.join(' | ')}`);

    // 5. diagnostics: a broken document must produce a rut-sourced error
    const broken = await vscode.workspace.openTextDocument({
        language: 'rut',
        content: 'fn broken(: nil {\n',
    });
    await vscode.window.showTextDocument(broken, { preview: true });
    const diags = await poll('diagnostics', async () => {
        const d = vscode.languages.getDiagnostics(broken.uri);
        return d.length > 0 ? d : null;
    });
    assert.strictEqual(diags[0].source, 'rut', `diag source: ${JSON.stringify(diags[0])}`);
    assert.strictEqual(diags[0].severity, vscode.DiagnosticSeverity.Error);
    log(`diagnostics: "${diags[0].message.slice(0, 48)}..." (source=rut)`);

    // 6. the dep walk — the crate's read-only walk through the injected env
    await depWalkChecks(log);

    // 7. nested projects — discovery feeds the walk (the multi-project lane)
    await nestedChecks(log);

    log('ALL CHECKS PASSED');
}

// ---- the dep walk (the store lane) ----
//
// A fixture workspace whose path dep lives OUTSIDE the folder boundary
// (invisible to findFiles, visible only to the walk) plus a url dep
// pinned by the workspace's `rut.lock` and served from the GLOBAL dep
// store — a temp `$RUT_HOME` the env adapter anchors the walk's store
// spelling at. The editor never fetches and never writes: a cold
// store is ONE error hint naming `rut install` (the rust-analyzer
// contract). Scenarios, in order (the miss MUST precede the hit — the
// wasm session keeps mounted indexes, so a not-mounted assertion is
// only honest before any mount):
//   (a) out-of-folder path dep completes (`use gadgets::` tier);
//   (b) the cold store → the one error hint naming `rut install`,
//       the dep never mounts (no store-path definition target);
//   (c) the store hit (a dep dir whose meta.json vouches the lock
//       pin) → mounted, go-to-definition lands in the store's real
//       `unpack/` file;
//   (d) auto-import: the bare `Wi` probe completes Widget with the
//       `use gadgets::Widget;` insert after the last leading `use`.
async function depWalkChecks(log) {
    const vscode = require('vscode');
    const REPO = join(__dirname, '..', '..', '..');
    const FIXTURES = join(__dirname, 'fixtures', 'dep-walk');

    // the url identity + its pin: the committed pouch zip's bytes
    // hash (the std-v18 lane; nothing reads the artifact here — the
    // store's meta vouches the pin; install-time verification is the
    // only hash pass). The probe name `VecPouch` exists ONLY in the
    // fixture store tree, so no std-twin can answer for it.
    const bundleBytes = fs.readFileSync(join(REPO, 'dist', 'std', 'pouch.zip'));
    const pin = crypto.createHash('sha256').update(bundleBytes).digest('hex');
    const url = 'https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v18/dist/std/pouch.zip';

    // the scratch world: ws/ becomes a workspace folder; outside/ sits
    // beside it — outside the folder boundary by construction; home/
    // is the temp global store the adapter anchors `.rut/deps/…` at
    const tmp = fs.mkdtempSync(join(os.tmpdir(), 'rut-depwalk-'));
    const ws = join(tmp, 'ws');
    fs.mkdirSync(ws);
    fs.cpSync(join(FIXTURES, 'outside'), join(tmp, 'outside'), { recursive: true });
    // the app package: root module at the default `src/mod.rut` (the
    // entry key is repealed — a bare name carries the default entry)
    fs.mkdirSync(join(ws, 'src'), { recursive: true });
    fs.copyFileSync(join(FIXTURES, 'workspace', 'src', 'mod.rut'), join(ws, 'src', 'mod.rut'));
    // the manifest carries NO pin (the inline sha256 key is retired);
    // the lock is the pin's only source. The salt comment renders
    // distinct bytes (a LEADING comment — every JSONC parser's first
    // lesson), so a re-render always fires the `**/rut.jsonc` watcher.
    const render = (salt) => {
        const template = fs.readFileSync(join(FIXTURES, 'workspace', 'rut.jsonc.template'), 'utf8');
        fs.writeFileSync(
            join(ws, 'rut.jsonc'),
            `// ${salt}\n${template.replaceAll('__POUCH_ROW__', `    "pouch": { "url": "${url}" }`)}`,
        );
        fs.writeFileSync(
            join(ws, 'rut.lock'),
            `${JSON.stringify(
                {
                    format: 'rutlock',
                    format_version: 1,
                    deps: [{ pkg: 'pouch', url, sha256: pin }],
                },
                null,
                2,
            )}\n`,
        );
    };

    // capture the hints (both lanes: the walk's notify → the error
    // messages, the mapping's def-drop → the info messages)
    const messages = [];
    const realError = vscode.window.showErrorMessage;
    const realInfo = vscode.window.showInformationMessage;
    vscode.window.showErrorMessage = (m) => {
        messages.push(String(m));
        return Promise.resolve(undefined);
    };
    vscode.window.showInformationMessage = (m) => {
        messages.push(String(m));
        return Promise.resolve(undefined);
    };

    const prevHome = process.env.RUT_HOME;
    try {
        process.env.RUT_HOME = join(tmp, 'home');
        render('cold');
        assert.ok(
            vscode.workspace.updateWorkspaceFolders(vscode.workspace.workspaceFolders.length, 0, {
                uri: vscode.Uri.file(ws),
            }),
            'adding the fixture workspace folder failed',
        );

        // (a) the OUT-OF-FOLDER path dep completes in the `use <name>::` tier
        const gadgetsProbe = await vscode.workspace.openTextDocument({
            language: 'rut',
            content: 'use gadgets::\nfn main() -> nil {\n}\n',
        });
        await poll('out-of-folder dep completion', async () => {
            const items = await vscode.commands.executeCommand(
                'vscode.executeCompletionItemProvider',
                gadgetsProbe.uri,
                new vscode.Position(0, 13),
            );
            const labels = items.map((i) => i.label);
            return labels.includes('Widget') && labels.includes('tag') ? labels : null;
        });
        log('dep walk: out-of-folder path dep `gadgets` completes');

        // (b) the cold store: the walk refused the url row — ONE error
        // hint naming `rut install` — and no store-path target exists
        // (the probe name only the fixture store could answer)
        const pouchLine = 'use pouch::{ VecPouch };';
        const pouchProbe = await vscode.workspace.openTextDocument({
            language: 'rut',
            content: `${pouchLine}\nfn main() -> nil {\n}\n`,
        });
        const at = pouchLine.indexOf('VecPouch');
        const storeTargets = async () => {
            const locs = await vscode.commands.executeCommand(
                'vscode.executeDefinitionProvider',
                pouchProbe.uri,
                new vscode.Position(0, at),
            );
            return locs.filter((l) => l.uri.fsPath.includes(join('deps', 'pouch@')));
        };
        const refusal = await poll(
            'cold-store hint',
            () => messages.find((x) => x.includes('not in the dep store')) || null,
        );
        assert.ok(
            refusal.includes('rut install'),
            `the refusal names the install lane: ${refusal}`,
        );
        assert.ok(refusal.includes(url), `the refusal names the url: ${refusal}`);
        assert.strictEqual(
            (await storeTargets()).length,
            0,
            'a cold store must never mount the dep',
        );
        log(`dep walk: cold store refused — "${refusal.slice(0, 60)}..."`);

        // (c) the store hit: publish the dep dir the walk derives
        // (`<name>@<pin16>` under `$RUT_HOME/deps` — rut-driver's
        // address, the same one `rut install` writes) with a meta.json
        // vouching the lock pin, then reload the deps through the
        // palette command — the dep mounts, go-to-definition lands in
        // the store's real `unpack/` file
        const depDir = join(process.env.RUT_HOME, 'deps', `pouch@${pin.slice(0, 16)}`);
        fs.mkdirSync(join(depDir, 'unpack', 'src'), { recursive: true });
        fs.writeFileSync(
            join(depDir, 'meta.json'),
            JSON.stringify({
                name: 'pouch',
                url,
                sha256: pin,
                format: 'zip',
                format_version: 1,
            }),
        );
        fs.writeFileSync(join(depDir, 'unpack', 'rut.jsonc'), JSON.stringify({ name: 'pouch' }));
        fs.writeFileSync(
            join(depDir, 'unpack', 'src', 'mod.rut'),
            'pub class VecPouch {\n    n: i32;\n}\n',
        );
        await vscode.commands.executeCommand('rut.reindexDeps');
        const mounted = await poll('store dep mounted (store go-to-def)', async () => {
            const targets = await storeTargets();
            return targets.length > 0 ? targets : null;
        });
        log(`dep walk: store dep mounted (${mounted[0].uri.fsPath.slice(-48)}...)`);

        // (d) auto-import: the bare `Wi` probe offers Widget with the
        // insert after the last leading `use`, sorted after the locals
        const autoProbe = await vscode.workspace.openTextDocument({
            language: 'rut',
            content: 'use gadgets::{ tag };\nfn main() -> nil {\n    let w = Wi;\n}\n',
        });
        const imp = await poll('auto-import item', async () => {
            const items = await vscode.commands.executeCommand(
                'vscode.executeCompletionItemProvider',
                autoProbe.uri,
                new vscode.Position(2, 14), // right after the `Wi` fragment
            );
            // the plain `class Widget` item and the auto-import item share
            // the label — the import offer is the one carrying the edit
            const item = items.find(
                (i) =>
                    i.label === 'Widget' &&
                    i.additionalTextEdits &&
                    i.additionalTextEdits.length === 1,
            );
            return item || null;
        });
        const edit = imp.additionalTextEdits[0];
        assert.strictEqual(
            edit.newText,
            '\nuse gadgets::Widget;',
            `the insert is a fresh line naming the module: ${JSON.stringify(edit.newText)}`,
        );
        assert.strictEqual(
            edit.range.start.line,
            0,
            'the insert rides after the last leading `use`',
        );
        assert.strictEqual(
            edit.range.start.line,
            edit.range.end.line,
            'the insert is one fresh line',
        );
        assert.ok(imp.sortText?.startsWith('~'), `the tier sorts after locals: ${imp.sortText}`);
        log('dep walk: auto-import inserts `use gadgets::Widget;` after the leading use');
    } finally {
        if (prevHome === undefined) {
            delete process.env.RUT_HOME;
        } else {
            process.env.RUT_HOME = prevHome;
        }
        vscode.window.showErrorMessage = realError;
        vscode.window.showInformationMessage = realInfo;
    }
}

// ---- the nested projects (the multi-project lane) ----
//
// A workspace folder whose ROOT carries no manifest, with nested rut
// projects inside — the ideal-toys shape that stranded every editor
// world before discovery: `reloadDeps` passed only the folder (the
// walk read `{root}/rut.jsonc`, found none, returned — nothing
// mounted) and the scan stamped nothing (the folder-root-only pkg
// refused the nested manifest). Discovery answers a SET: every
// `**/rut.jsonc` under the shared ignore set is a project, the walk
// loops roots, the collection dedupes first-mount-wins. The fixtures
// render from templates (the dep-walk trick — the in-repo copies stay
// inert), then the folder add rides the change hook's full re-run
// (re-discovery + re-scan).
//
// Assertions, through the REAL editor surfaces, in EACH project's doc:
//   (a) `let x = y;` squiggles `unknown name` (the check lane runs —
//       the doc's own project mounted),
//   (b) hover on `log.info`'s `info` resolves (the ink path dep
//       mounted from the nested root),
//   (c) the MULTI pin: both docs answer independently — sibling's
//       squiggle sits on sibling's own line (connector's range never
//       leaks), and each doc's identical `tag` local defines into its
//       OWN file,
//   (d) a manifest under `target/` is never a project: the trap doc
//       stays parse-only (no unknown-name squiggle),
//   (e) the same-name twin is non-fatal: first-mount-wins, the twin's
//       own doc still answers,
//   (f) a project created MID-SESSION joins the root set (the
//       manifest watcher invalidates discovery; the reload walks it).
async function nestedChecks(log) {
    const vscode = require('vscode');
    const FIXTURES = join(__dirname, 'fixtures', 'nested');
    const tmp = fs.mkdtempSync(join(os.tmpdir(), 'rut-nested-'));
    const root = join(tmp, 'ws'); // the folder root: NO manifest here
    fs.mkdirSync(root);
    fs.cpSync(FIXTURES, root, { recursive: true });
    // the in-repo manifests are `.template`s — the rendered copies in
    // the tmp folder are the live projects
    const render = (dir) => {
        const template = fs.readFileSync(join(root, dir, 'rut.jsonc.template'), 'utf8');
        fs.writeFileSync(join(root, dir, 'rut.jsonc'), template);
    };
    for (const dir of ['connector', 'sibling', 'ink', 'twin']) {
        render(dir);
    }

    // a needle's 0-based position in `text`, offset by `offset` chars
    const posOf = (text, needle, offset = 0) => {
        const at = text.indexOf(needle);
        assert.ok(at >= 0, `probe ${JSON.stringify(needle)} missing from the fixture`);
        const lineStart = text.lastIndexOf('\n', at) + 1;
        return new vscode.Position(
            text.slice(0, at).split('\n').length - 1,
            at - lineStart + offset,
        );
    };
    const unknownY = (uri) =>
        vscode.languages
            .getDiagnostics(uri)
            .filter((d) => d.message.includes('unknown name') && d.message.includes('`y`'));

    assert.ok(
        vscode.workspace.updateWorkspaceFolders(vscode.workspace.workspaceFolders.length, 0, {
            uri: vscode.Uri.file(root),
        }),
        'adding the nested fixture folder failed',
    );

    // open BOTH project docs (real files under the new folder)
    const aPath = join(root, 'connector', 'src', 'mod.rut');
    const bPath = join(root, 'sibling', 'src', 'mod.rut');
    const aUri = vscode.Uri.file(aPath);
    const bUri = vscode.Uri.file(bPath);
    const aText = fs.readFileSync(aPath, 'utf8');
    const bText = fs.readFileSync(bPath, 'utf8');
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(aUri), {
        preview: true,
    });
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(bUri), {
        preview: true,
    });

    // (a) each doc squiggles its own `y` — the scan's ownership stamp
    // (nearest manifest dir) is what turns the check lane on
    const yA = posOf(aText, 'let x = y;', 8); // the `y` token
    const yB = posOf(bText, 'let x = y;', 8);
    assert.notStrictEqual(
        yA.line,
        yB.line,
        'fixture layout: the probes must sit on distinct lines',
    );
    const aDiags = await poll('connector squiggle', async () => {
        const d = unknownY(aUri);
        return d.length > 0 ? d : null;
    });
    const bDiags = await poll('sibling squiggle', async () => {
        const d = unknownY(bUri);
        return d.length > 0 ? d : null;
    });
    assert.strictEqual(aDiags[0].source, 'rut', `diag source: ${JSON.stringify(aDiags[0])}`);
    assert.strictEqual(
        aDiags[0].range.start.line,
        yA.line,
        `connector's y squiggle sits on its own line: ${JSON.stringify(aDiags[0].range)}`,
    );
    assert.strictEqual(
        bDiags[0].range.start.line,
        yB.line,
        `sibling's y squiggle sits on its own line: ${JSON.stringify(bDiags[0].range)}`,
    );
    assert.ok(
        bDiags.every((d) => d.range.start.line !== yA.line),
        "connector's y range leaked into sibling's diagnostics",
    );
    log(`nested: connector y-squiggle (line ${yA.line}), sibling y-squiggle (line ${yB.line})`);

    // (b) hover resolves the ink dep's impl fn in EACH project (the
    // nested root mounted its path dep like any root does)
    const hoverInfo = async (uri, text) => {
        const at = posOf(text, 'log.info', 4); // the `info` member
        const h = await vscode.commands.executeCommand('vscode.executeHoverProvider', uri, at);
        return h && h.length > 0 ? h[0] : null;
    };
    for (const [name, uri, text] of [
        ['connector', aUri, aText],
        ['sibling', bUri, bText],
    ]) {
        const h = await poll(`${name} ink hover`, () => hoverInfo(uri, text));
        const value = h.contents.map((c) => c.value).join('\n');
        assert.ok(
            value.includes('fn info'),
            `${name}'s log.info hover is not ink's impl fn: ${value.slice(0, 80)}`,
        );
    }
    log('nested: log.info hovers resolve into the ink dep from both projects');

    // (c) the identical locals hold: each doc's `tag` usage defines
    // into its OWN file — the per-doc entrypoint + per-file stamps
    // hold under N=2
    const tagA = posOf(aText, 'let said = tag;', 11); // the `tag` token
    const tagB = posOf(bText, 'let said = tag;', 11);
    const defTargets = async (uri, at) =>
        vscode.commands.executeCommand('vscode.executeDefinitionProvider', uri, at);
    const defA = await poll('connector tag definition', () => defTargets(aUri, tagA));
    const defB = await poll('sibling tag definition', () => defTargets(bUri, tagB));
    assert.ok(
        defA.some((l) => l.uri.fsPath === aPath),
        `connector's tag defines into its own file: ${JSON.stringify(defA)}`,
    );
    assert.ok(
        defB.some((l) => l.uri.fsPath === bPath),
        `sibling's tag defines into its own file: ${JSON.stringify(defB)}`,
    );
    log('nested: the identical `tag` locals each define into their own project');

    // (d) a manifest under `target/` is never a project — the trap doc
    // stays unstamped, parse-only: no unknown-name squiggle ever (the
    // analysis lanes are proved live by the squiggles above)
    const trapUri = vscode.Uri.file(join(root, 'target', 'src', 'mod.rut'));
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(trapUri), {
        preview: true,
    });
    await sleep(1000); // a generous settle — the negative must hold
    assert.strictEqual(
        unknownY(trapUri).length,
        0,
        'a manifest under target/ must never be a project',
    );
    log('nested: the target/ trap stayed invisible (parse-only)');

    // (e) the same-name twin: first-mount-wins is non-fatal — the
    // twin's own doc still answers its probe
    const twinPath = join(root, 'twin', 'src', 'mod.rut');
    const twinUri = vscode.Uri.file(twinPath);
    const twinText = fs.readFileSync(twinPath, 'utf8');
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(twinUri), {
        preview: true,
    });
    const yTwin = posOf(twinText, 'let x = y;', 8);
    const twinDiags = await poll('twin squiggle', async () => {
        const d = unknownY(twinUri);
        return d.length > 0 ? d : null;
    });
    assert.strictEqual(
        twinDiags[0].range.start.line,
        yTwin.line,
        `the twin doc answers its own probe: ${JSON.stringify(twinDiags[0].range)}`,
    );
    log('nested: the same-name twin is non-fatal (its doc answers)');

    // (f) a project created MID-SESSION joins the root set: the
    // manifest watcher invalidates discovery, the reload walks the new
    // root, `use late::` completes its surface. The palette command
    // rides as the sanctioned catch-up for watcher or search-index lag.
    fs.mkdirSync(join(root, 'late', 'src'), { recursive: true });
    fs.writeFileSync(
        join(root, 'late', 'rut.jsonc'),
        '{\n    // late — created mid-session\n    "name": "late"\n}\n',
    );
    fs.writeFileSync(
        join(root, 'late', 'src', 'mod.rut'),
        'pub fn late_fn() -> i32 {\n    return 7;\n}\n',
    );
    const lateProbe = await vscode.workspace.openTextDocument({
        language: 'rut',
        content: 'use late::\nfn main() -> nil {\n}\n',
    });
    let mounted = false;
    for (let i = 0; i < 60 && !mounted; i++) {
        if (i === 15) {
            await vscode.commands.executeCommand('rut.reindexDeps');
        }
        await sleep(200);
        const items = await vscode.commands.executeCommand(
            'vscode.executeCompletionItemProvider',
            lateProbe.uri,
            new vscode.Position(0, 10), // right after `use late::`
        );
        mounted = items.map((x) => x.label).includes('late_fn');
    }
    assert.ok(mounted, 'a manifest created mid-session must join the root set');
    log('nested: the mid-session project `late` mounted (discovery re-ran)');
}

module.exports = { run };
