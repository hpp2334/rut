// The workspace scan's PURE tier — the path math of project discovery
// and ownership, compiled straight from src/ (no vscode, no wasm, no
// VS Code host). Pins the multi-project cases the host suite
// (test/runTest.js, the `nested` fixture) drives through real windows:
// sibling manifests, a root manifest beside nested ones, the folder
// bound, and the shared ignore set. The glob itself lives in VS Code's
// findFiles — the constant is the pin here; the invisible-trap
// behavior rides the host suite's `target/` fixture.

const assert = require('node:assert');
const esbuild = require('esbuild');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const EXT = path.join(__dirname, '..');

// the pure modules, compiled on the spot (esbuild is a devDependency;
// the same compile the extension bundle rides)
const outDir = fs.mkdtempSync(path.join(os.tmpdir(), 'rut-units-'));
esbuild.buildSync({
    entryPoints: [path.join(EXT, 'src', 'ownership.ts'), path.join(EXT, 'src', 'constants.ts')],
    outdir: outDir,
    bundle: true,
    format: 'cjs',
    platform: 'node',
    target: 'node18',
    logLevel: 'silent',
});
const { mergeProjectDirs, ownerDir, stripSrcShell, resolveHostPath } = require(
    path.join(outDir, 'ownership.js'),
);
const { IGNORED } = require(path.join(outDir, 'constants.js'));
fs.rmSync(outDir, { recursive: true, force: true });

// ---- the shared ignore set: both scans pass the SAME glob, and
// target/ is named (a manifest under target/ is never a project)
assert.strictEqual(
    IGNORED,
    '{**/target/**,**/node_modules/**,**/.git/**,**/.*/**}',
    'the scans share one ignore set',
);

// ---- the discovery merge: the folder roots first, then the found
// manifest dirs, deduped in first-seen order (a root manifest and a
// scan hit for the same dir are ONE project; sibling projects never
// shadow each other)
assert.deepStrictEqual(
    mergeProjectDirs(
        ['/ws', '/other'],
        ['/ws/connector', '/ws/sibling', '/other', '/ws/connector'],
    ),
    ['/ws', '/other', '/ws/connector', '/ws/sibling'],
    'folders first, then the found dirs, deduped',
);
assert.deepStrictEqual(mergeProjectDirs([], []), [], 'nothing found, nothing discovered');

// ---- ownership: the NEAREST enclosing manifest dir wins, walking
// from the file's dir up to the folder root (inclusive)
const projects = new Set(['/ws', '/ws/connector', '/ws/connector/src/deep', '/ws/sibling']);

// the file's own dir counts (inclusive of the file's dir)
assert.strictEqual(ownerDir('/ws', '/ws/connector/src/mod.rut', projects), '/ws/connector');
// the nearest ancestor, not the farther one
assert.strictEqual(
    ownerDir('/ws', '/ws/connector/src/deep/mod.rut', projects),
    '/ws/connector/src/deep',
);
// no manifest anywhere on the walk — no owner (the root has no
// manifest in this set, and neither does the file's tree)
assert.strictEqual(
    ownerDir('/ws', '/ws/plain/src/mod.rut', new Set(['/ws/connector', '/ws/sibling'])),
    null,
);

// the folder root counts as an owner only when IT has a manifest
assert.strictEqual(ownerDir('/ws', '/ws/src/mod.rut', projects), '/ws');
assert.strictEqual(ownerDir('/ws', '/ws/src/mod.rut', new Set(['/ws/connector'])), null);
// a manifest ABOVE the folder never owns — the folder bounds the walk
assert.strictEqual(ownerDir('/ws/nested', '/ws/nested/src/mod.rut', new Set(['/ws'])), null);

// ---- the multi-project cases ----
// two sibling manifests: each file's owner is its own project
assert.strictEqual(ownerDir('/ws', '/ws/connector/src/a/mod.rut', projects), '/ws/connector');
assert.strictEqual(ownerDir('/ws', '/ws/sibling/src/a/mod.rut', projects), '/ws/sibling');
// a manifest at the folder root BESIDE nested ones: the root wins for
// its own files, the nested ones own theirs
assert.strictEqual(ownerDir('/ws', '/ws/src/mod.rut', projects), '/ws');
assert.strictEqual(ownerDir('/ws', '/ws/connector/src/mod.rut', projects), '/ws/connector');

// ---- the src/ shell: the mod path spelling
assert.strictEqual(stripSrcShell('src'), '');
assert.strictEqual(stripSrcShell('src/layout'), 'layout');
assert.strictEqual(stripSrcShell('src/layout/grid'), 'layout/grid');
assert.strictEqual(stripSrcShell(''), null); // the root beside the manifest
assert.strictEqual(stripSrcShell('pages'), null); // outside the shell

// ---- the env path law: the `file://` arm (the mod-name completion
// probe hands the open document's directory verbatim), the plain-path
// normalize, and the `.rut/` store anchor
{
    const decoded = resolveHostPath('file:///ws/a%20b/src/mod.rut');
    assert.ok(path.isAbsolute(decoded), `file:// resolves absolute: ${decoded}`);
    assert.ok(!decoded.includes('%20'), `file:// decodes percent escapes: ${decoded}`);
    assert.strictEqual(decoded, '/ws/a b/src/mod.rut');
    assert.strictEqual(
        resolveHostPath('/ws/./src/../src/mod.rut'),
        '/ws/src/mod.rut',
        'a plain rooted path still normalizes',
    );
    process.env.RUT_HOME = path.join(os.tmpdir(), 'rut-units-home');
    assert.strictEqual(
        resolveHostPath('.rut/deps/kit@abc0123456789abcd/unpack/src/mod.rut'),
        path.join(process.env.RUT_HOME, 'deps/kit@abc0123456789abcd/unpack/src/mod.rut'),
        '.rut/ still anchors at the real $RUT_HOME',
    );
}

console.log('workspace units: all ownership/discovery/path-law pins hold');
