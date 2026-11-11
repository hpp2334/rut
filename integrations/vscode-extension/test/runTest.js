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
const FIXTURE = join(__dirname, 'fixtures', 'symbols', 'symbols.rut');

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

    // the url identity + its pin: the committed pouch bundle's bytes
    // hash (nothing reads the artifact here — the store's meta vouches
    // the pin; install-time verification is the only hash pass). The
    // probe name `VecPouch` exists ONLY in the fixture store tree, so
    // no std-twin can answer for it.
    const bundleBytes = fs.readFileSync(join(REPO, 'dist', 'std', 'pouch.rutbundle'));
    const pin = crypto.createHash('sha256').update(bundleBytes).digest('hex');
    const url = 'https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v15/dist/std/pouch.rutbundle';

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
                format: 'rutbundle',
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

module.exports = { run };
