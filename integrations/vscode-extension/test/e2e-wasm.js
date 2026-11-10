// Through-wasm e2e gate — the extension-side twin of
// crates/rut-lsp-wasm/tests/corpus.rs (the lsp-align survey §3.6 design).
// Drives the SHIPPED bin/rut-lsp.wasm over the full repo corpus
// (rut/ + examples/ + demo/src/examples/ + benches/workloads/) through
// the extension's OWN ABI binding (src/wasm.ts, bundled to out/wasm.js
// by esbuild.mjs) — the exact load path src/extension.ts uses, minus the
// vscode host. Zero false diagnostics is the alignment lock: the wasm
// module and any other face share the one crate's queries, so this
// gate cannot pass while the shipped artifact and the language drift.
//
// Plus the hover/completion smoke the survey demanded: a `?T` hover
// (the RFC 0043 alias rendering phase 1 fixed), member hover/completion
// through a `?Circle` let-binding (the ty_head fix), and the ambient
// core completion (the prelude's plain bare rows) — all against the
// shipped binary.
//
// Phase 2 (lsp-features) adds the DEFINITION round trips: request
// position → response span == the declaring ident EXACTLY, through all
// three resolution layers — within-file (binding pass + decl layer),
// cross-file (the use graph into a rut.add_def'ed module), and stdlib
// (a STORE-MOUNTED dep's true source file) — plus a typeDefinition
// smoke. A feature without a gate through the shipped artifact does
// not land.
//
// Phase 3 (lsp-features) adds the INLAY smokes: type hints on
// unannotated bindings and param-name hints at exact-arity call sites,
// resolved through the REBUILT artifact at known corpus positions —
// each label checked against the source's own ground truth (an
// annotation or a decl-site type line in the same file, or the
// stdlib's true mod.rut decl) — plus the hover-consistency law (the
// hint's type text is what hover shows for the same binding) and the
// annotated-lets-never-restated + range-filter laws on a synthetic doc.
//
// Phase 4 (lsp-features) adds the REFERENCES + SIGNATURE-HELP smokes:
// references round trips (a decl -> the EXACT expected use-position
// set, shadow-aware across scopes, and cross-file through the use
// graph's reverse edges in BOTH directions), and signature help
// (verbatim signature + active parameter correct at first/middle/
// nested positions; an arity mismatch -> no help).
//
// Phase 5 (manifest-deps) added the DEP ABI; today's shape is the
// three-lane law: the provider queries, the workspace-scan push
// (rut.add_def / rut.add_def_mod / rut.parse_manifest), and the dep
// pull (set_env + rut.resolve_deps — the read-only walk over the
// injected env). The std-zips phase then moved std OFF the ambient
// wasm onto the dep lane for good: the core prelude is the ONE
// ambient index, every other std pkg reaches the editor exactly the
// way `rut new` scaffolds it — a manifest row, a lock pin, and the
// global store — which this gate serves through a synthesized STORE
// fixture (deps/<name>@<sha256[:16]>/meta.json + the unpack/ tree,
// the urls and pins derived from the CLI template's own pin table).
// No raw-ABI exports exist anymore; every lane below goes through
// the extension's binding.
//
// Runs headless in plain node — no `code` needed; the Extension Host
// suite (test:host) stays the loud-skip lane on machines without VS Code.

const assert = require('node:assert');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const EXT = path.join(__dirname, '..');
const REPO = path.join(EXT, '..', '..');
const WASM_PATH = path.join(EXT, 'bin', 'rut-lsp.wasm');
const BINDING_PATH = path.join(EXT, 'out', 'wasm.js');
const FIXTURE = path.join(__dirname, 'fixtures', 'symbols', 'src', 'mod.rut');

// corpus roots, mirroring crates/rut-lsp-wasm/tests/corpus.rs and
// test/grammar-corpus.js
const CORPUS_ROOTS = [
    'rut',
    'examples',
    path.join('demo', 'src', 'examples'),
    path.join('benches', 'workloads'),
];
const MIN_CORPUS_FILES = 50; // 53 today — guards against a silently empty walk

function* rutFiles(dir) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const p = path.join(dir, entry.name);
        if (entry.isDirectory()) yield* rutFiles(p);
        else if (entry.name.endsWith('.rut')) yield p;
    }
}

// [line, character] (0-based, LSP order) of the n-th occurrence of needle
function posOf(src, needle, occurrence = 1) {
    let at = -1;
    for (let i = 0; i < occurrence; i++) at = src.indexOf(needle, at + 1);
    assert.ok(at >= 0, `needle ${JSON.stringify(needle)} not found`);
    const before = src.slice(0, at);
    return [before.split('\n').length - 1, at - (before.lastIndexOf('\n') + 1)];
}

// [line, character] of needle on the first line matching lineRe —
// immune to the same word appearing in comment prose above the code
function posOnLine(src, lineRe, needle) {
    const lines = src.split('\n');
    for (let i = 0; i < lines.length; i++) {
        if (!lineRe.test(lines[i])) continue;
        const ch = lines[i].indexOf(needle);
        assert.ok(ch >= 0, `${JSON.stringify(needle)} not on line matching ${lineRe}: ${lines[i]}`);
        return [i, ch];
    }
    assert.ok(false, `no line matches ${lineRe}`);
}

async function main() {
    if (!fs.existsSync(WASM_PATH)) {
        console.error(
            `e2e-wasm: bin/rut-lsp.wasm not found — this gate drives the SHIPPED ` +
                `artifact; build it first:\n  cd integrations/vscode-extension && npm run build:wasm`,
        );
        process.exit(1);
    }
    if (!fs.existsSync(BINDING_PATH)) {
        console.error(
            `e2e-wasm: ${path.relative(REPO, BINDING_PATH)} not found — run \`npm run compile\` first`,
        );
        process.exit(1);
    }
    // the extension's own binding (the wasm crate's js face, bundled by
    // src/wasm.ts), not a copy
    const { RutLsp } = require(BINDING_PATH);
    const rut = await RutLsp.load(fs.readFileSync(WASM_PATH));
    const bad = [];
    let smokes = 0;

    // ---- the capability probe (the stale-install seam) ----
    // The old probe checked `hasSrczip`/`hasBundleExtract` on the loaded
    // artifact, because a same-version-but-older install could pair a new
    // binding with an old wasm and dead-end every std ctrl+click. The
    // wasm-bindgen glue and the .wasm now ship as ONE pair (one build,
    // one vsix) — the module cannot be older than its lane, and a
    // byte-mismatched pair fails `load` loudly instead. The boot itself
    // is the probe.
    smokes++;

    // ---- the semantic-token legend (16 types, the extension's map —
    // comments joined the classes, phase 5 added the namespace class) ----
    const legend = await rut.legend();
    if (
        !(
            legend.length === 16 &&
            legend.includes('enumMember') &&
            legend.includes('keyword') &&
            legend.includes('comment') &&
            legend.includes('namespace')
        )
    ) {
        bad.push(`legend drifted: ${JSON.stringify(legend)}`);
    }
    smokes++;

    // ---- the corpus: zero false diagnostics through the SHIPPED wasm ----
    // (the pre-align artifact flagged 15 of 53 files with 1,796 false
    // diagnostics — every ?T site read `expected a type name`)
    // THIS LOCK RUNS FIRST — ahead of every fail-fast (assert) lane that
    // follows — so a drifted pin or a stale artifact can never die the
    // gate early and mask the alignment verdict: a corpus violation lands
    // in `bad` even when a later lane would have thrown first. (A
    // hardcoded std-v8 URL pin once sat ahead of this block and did
    // exactly that masking.)
    const files = [...rutFiles(path.join(REPO, 'rut'))];
    for (const root of CORPUS_ROOTS.slice(1)) files.push(...rutFiles(path.join(REPO, root)));
    files.sort();
    assert.ok(
        files.length >= MIN_CORPUS_FILES,
        `corpus walk found only ${files.length} .rut files (expected >= ${MIN_CORPUS_FILES})`,
    );

    let symbols = 0;
    for (const file of files) {
        const src = fs.readFileSync(file, 'utf8');
        // the absolute path AS the uri — the .d.rut suffix survives, so the
        // module's mode_of picks Decl for the decl surfaces (core.d.rut &
        // co.); a mode mistake here would light them up as diagnostics.
        const uri = `file://${file}`;
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `${path.relative(REPO, file)}: ${a.diags.length} false diagnostic(s), first: ${JSON.stringify(a.diags[0].message)}`,
            );
            continue;
        }
        if (!(a.tokens.data && a.tokens.data.length > 0)) {
            bad.push(`${path.relative(REPO, file)}: empty token stream`);
        }
        if (!path.basename(file).endsWith('.d.rut')) {
            // Impl-mode files yield document symbols (corpus_classifies' law)
            if (!a.symbols.length) {
                bad.push(`${path.relative(REPO, file)}: no document symbols`);
            }
            symbols += a.symbols.length;
        }
    }

    // ---- the fixture: a current-grammar sample, analyzed clean ----
    const fixtureSrc = fs.readFileSync(FIXTURE, 'utf8');
    const fixtureUri = `file://${FIXTURE}`;
    const fx = await rut.analyze(fixtureUri, fixtureSrc);
    if (fx.diags.length !== 0) {
        bad.push(`fixture has ${fx.diags.length} diagnostic(s): ${JSON.stringify(fx.diags[0])}`);
    }
    if (!(fx.tokens.data && fx.tokens.data.length >= 100)) {
        bad.push(`fixture token stream thin: ${fx.tokens.data?.length}`);
    }
    const fxNames = fx.symbols.map((s) => s.name);
    for (const want of [
        'Color',
        'Point',
        'Circle',
        'Drawable',
        'main',
        'Slot',
        'tag',
        'kind',
        'widen',
    ]) {
        if (!fxNames.includes(want))
            bad.push(`fixture symbol ${want} missing (got ${fxNames.join(', ')})`);
    }
    smokes++;

    // ---- ?T hover: the RFC 0043 alias renders its nullable target ----
    // (M4's ty_src arm — the pre-align builds printed `type Maybe = ;`)
    {
        const [line, ch] = posOnLine(fixtureSrc, /^\s*type Maybe = /, 'Maybe');
        const h = await rut.hover(fixtureUri, line, ch);
        if (!h?.contents.value.includes('type Maybe = ?i32;')) {
            bad.push(
                `alias hover lost the ?T target: ${h && JSON.stringify(h.contents.value.slice(0, 80))}`,
            );
        }
        smokes++;
    }
    // the RFC 0004 primitives hover their current `primitive` form
    // (the stale e41915d binary said `builtin str`)
    {
        const [line, ch] = posOnLine(fixtureSrc, /^\s*fn tag\(s: str/, 'str');
        const h = await rut.hover(fixtureUri, line, ch);
        if (!h?.contents.value.includes('primitive str {')) {
            bad.push(
                `str hover is not the primitive form: ${h && JSON.stringify(h.contents.value.slice(0, 60))}`,
            );
        }
        smokes++;
    }

    // ---- member hover/completion through a `?Circle` let-binding ----
    // (M4's ty_head arm — mirrors crates/rut-lsp-wasm/tests/hover.rs's
    // nullable_let_binding_resolves_members through the shipped wasm;
    // methods live in impl blocks — RFC 0012 — so the doc analyzes clean)
    {
        const uri = 'file:///ws/e2e-nullable.rut';
        const src = [
            'class Circle {',
            '    r: f64;',
            '}',
            'impl Circle {',
            '    fn area(self) -> f64 { return 3.14; }',
            '}',
            'fn go() -> f64 {',
            '    let c: ?Circle = nil;',
            '    return c.area();',
            '}',
            '',
        ].join('\n');
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `nullable probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const [hline, hch] = posOf(src, 'area', 2); // the call site, not the decl
        const h = await rut.hover(uri, hline, hch);
        if (
            !h?.contents.value.includes('fn area(self) -> f64') ||
            !h.contents.value.includes('in `Circle`')
        ) {
            bad.push(
                `?Circle member hover missed: ${h && JSON.stringify(h.contents.value.slice(0, 80))}`,
            );
        }
        const [cline, cch] = posOf(src, 'c.');
        const items = await rut.complete(uri, cline, cch + 2, src);
        const labels = items.map((i) => i.label);
        for (const want of ['area', 'r']) {
            if (!labels.includes(want))
                bad.push(`?Circle completion lacks '${want}' (got ${labels.join(', ')})`);
        }
        smokes++;
    }

    // ---- the ambient lane is the PRELUDE alone (std rides the dep
    // lane now): a lone projectless doc gets the ambient core rows —
    // StackTrace and panic complete PLAIN (`prelude builtin` binds in
    // every unit, no import edit rides the item) — and NOTHING else
    // from std: no Vec, no HashMap, no Logger. The honest none is the
    // point (a lone file declares no deps, exactly like any
    // third-party lib); the mounted world below lights std up. ----
    {
        const uri = 'file:///ws/e2e-ambient.rut';
        const src = 'fn main() -> nil {\n}\n';
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `ambient probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const bareItems = await rut.complete(uri, 0, 0, src);
        const labels = bareItems.map((i) => i.label);
        // the ambient core rows stay PLAIN bare offers (the `prelude
        // builtin` spellings bind in every unit — no import edit on them)
        for (const want of ['StackTrace', 'panic']) {
            const hit = bareItems.find((i) => i.label === want);
            if (!hit) bad.push(`bare completion lacks the ambient core row '${want}'`);
            else if ((hit.additionalTextEdits || []).length !== 0) {
                bad.push(`ambient core row '${want}' must complete plain, no import edit`);
            }
        }
        // std is NOT ambient: none of its names complete for a doc that
        // declares no deps
        for (const absent of ['Vec', 'HashMap', 'Logger', 'map_hput']) {
            if (labels.includes(absent))
                bad.push(`bare completion offers std's '${absent}' — std is not ambient`);
        }
        smokes++;
    }

    // ---- the std STORE world (std rides the dep lane): the project
    // shape `rut new` scaffolds — a manifest declaring url deps, a
    // lock pinning them, and the global store the walk reads. The
    // fixture synthesizes the store exactly as `rut install` lays it:
    // deps/<name>@<sha256[:16]>/ with meta.json vouching the FULL
    // sha256 of the COMMITTED dist/std zip (the bytes the url names —
    // asserted, so a stale mirror cannot serve quietly) and the
    // unpack/ tree carrying the repo sources the zips pack. The urls
    // and pins derive from the CLI template's own pin table, so a std
    // re-cut moves this gate with it. ----
    const stdZipPath = (name) => path.join(REPO, 'dist', 'std', `${name}.zip`);
    const stdPins = (() => {
        const manifestText = fs.readFileSync(
            path.join(REPO, 'crates', 'rut-cli', 'template', 'rut.jsonc'),
            'utf8',
        );
        const lockText = fs.readFileSync(
            path.join(REPO, 'crates', 'rut-cli', 'template', 'rut.lock'),
            'utf8',
        );
        const pins = {};
        for (const name of ['pouch', 'ink']) {
            const url = manifestText.match(
                new RegExp(`"${name}"\\s*:\\s*\\{\\s*"url"\\s*:\\s*"([^"]+)"`),
            )[1];
            const row = lockText.match(new RegExp(`\\{[^{}]*"pkg"\\s*:\\s*"${name}"[^{}]*\\}`))[0];
            pins[name] = { url, sha256: row.match(/"sha256"\s*:\s*"([0-9a-f]{64})"/)[1] };
        }
        return pins;
    })();
    const POUCH_STORE_DIR = `.rut/deps/pouch@${stdPins.pouch.sha256.slice(0, 16)}`;
    const stdStoreFiles = (() => {
        const files = new Map();
        files.set(
            '/ws/stdproj/rut.lock',
            JSON.stringify(
                {
                    format: 'rutlock',
                    format_version: 1,
                    deps: Object.entries(stdPins).map(([pkg, { url, sha256 }]) => ({
                        pkg,
                        url,
                        sha256,
                    })),
                },
                null,
                2,
            ),
        );
        files.set(
            '/ws/stdproj/rut.jsonc',
            JSON.stringify({
                name: 'stdproj',
                deps: Object.fromEntries(
                    Object.entries(stdPins).map(([n, { url }]) => [n, { url }]),
                ),
            }),
        );
        files.set('/ws/stdproj/src/mod.rut', 'fn main() -> nil {\n}\n');
        for (const [name, { url, sha256 }] of Object.entries(stdPins)) {
            const dir = `.rut/deps/${name}@${sha256.slice(0, 16)}`;
            files.set(
                `${dir}/meta.json`,
                JSON.stringify({ name, url, sha256, format: 'zip', format_version: 1 }),
            );
            if (name === 'ink') {
                // the unpack law: ink's zip carries its host dep as an
                // embedded group under the extracted tree, and ink's own
                // manifest points its path row into it — the fixture
                // mirrors the shape (the group's bytes are the repo's
                // own ink_host sources, what the zip packs)
                files.set(
                    `${dir}/unpack/rut.jsonc`,
                    JSON.stringify({
                        name: 'ink',
                        deps: { ink_host: { path: 'deps/ink_host' } },
                    }),
                );
                files.set(
                    `${dir}/unpack/deps/ink_host/rut.jsonc`,
                    JSON.stringify({
                        name: 'ink_host',
                        type: 'host',
                        entry: { type: './ink_host.d.rut' },
                    }),
                );
                files.set(
                    `${dir}/unpack/deps/ink_host/src/ink_host.d.rut`,
                    fs.readFileSync(
                        path.join(REPO, 'rut', 'ink_host', 'src', 'ink_host.d.rut'),
                        'utf8',
                    ),
                );
            } else {
                files.set(`${dir}/unpack/rut.jsonc`, JSON.stringify({ name }));
            }
            files.set(
                `${dir}/unpack/src/mod.rut`,
                fs.readFileSync(path.join(REPO, 'rut', name, 'src', 'mod.rut'), 'utf8'),
            );
        }
        return files;
    })();
    const makeStdEnv = () => {
        const enc = new TextEncoder();
        return {
            read(p) {
                const v = stdStoreFiles.get(p);
                return v === undefined ? null : enc.encode(v);
            },
            readDir(p) {
                const names = [...stdStoreFiles.keys()]
                    .filter((k) => k.startsWith(`${p}/`))
                    .map((k) => k.slice(p.length + 1).split('/')[0]);
                return names.length ? [...new Set(names)] : null;
            },
            isDir(p) {
                return [...stdStoreFiles.keys()].some((k) => k.startsWith(`${p}/`));
            },
            notify() {},
        };
    };
    let std;
    {
        // the store vouches the pins: the committed zips' bytes hash to
        // the lock pins the fixture serves (the same law rut install
        // verifies — a drifted mirror dies here, not in a test below)
        for (const [name, { sha256 }] of Object.entries(stdPins)) {
            assert.strictEqual(
                crypto
                    .createHash('sha256')
                    .update(fs.readFileSync(stdZipPath(name)))
                    .digest('hex'),
                sha256,
                `dist/std/${name}.zip matches the template pin`,
            );
        }
        std = await RutLsp.load(fs.readFileSync(WASM_PATH), makeStdEnv());
        // the extension's own sequence: the manifest table first, then
        // the walk mounts the store's world into the same defs store
        const table = await std.parseManifest(stdStoreFiles.get('/ws/stdproj/rut.jsonc'));
        assert.ok(!table.error && table.name === 'stdproj', `manifest: ${JSON.stringify(table)}`);
        assert.strictEqual(
            table.deps.filter((r) => r.source.kind === 'url').length,
            2,
            `the url rows: ${JSON.stringify(table.deps)}`,
        );
        const wout = await std.resolveDeps(['/ws/stdproj']);
        assert.ok(
            !wout.error && wout.missing.length === 0,
            `the std store mounts clean: ${JSON.stringify(wout)}`,
        );
        // std answers through the mounted world: the module tier lists
        // the deps, the auto-import tier offers Vec (pouch is a DIRECT
        // dep of the project, so the use insert rides the item)
        const projUri = 'file:///ws/stdproj/src/mod.rut';
        const projSrc = stdStoreFiles.get('/ws/stdproj/src/mod.rut');
        await std.analyze(projUri, projSrc);
        const bare = await std.complete(projUri, 0, 0, projSrc);
        const bareLabels = bare.map((i) => i.label);
        if (!bareLabels.includes('Vec'))
            bad.push(
                `mounted-world bare completion lacks pouch's 'Vec' (${bareLabels.length} items)`,
            );
        const vec = bare.find((i) => i.label === 'Vec');
        if (vec && !/^pouch::Vec — import$/.test(vec.detail || '')) {
            bad.push(`mounted 'Vec' is not the import offer: '${vec.detail}'`);
        }
        smokes++;
    }

    // ---- plan3 phase 2 (A3): the TYPE TIER at an annotation boundary —
    // `let v: u⏐` offers the primitive table (u8..u64), never the
    // keyword table, fns, or locals; and (A6 pin) the brace form of a
    // use — `use pouch::{ ⏐` — offers the module's exports. ----
    {
        const uri = 'file:///ws/e2e-type-tier.rut';
        const src = 'fn go() -> nil {\n    let v: u\n}\n';
        const items = await rut.complete(uri, 1, '    let v: u'.length, src);
        const labels = items.map((i) => i.label);
        for (const want of ['u8', 'u16', 'u32', 'u64']) {
            if (!labels.includes(want))
                bad.push(`type tier lacks '${want}' (got ${labels.join(', ')})`);
        }
        for (const absent of ['fn', 'let', 'go']) {
            if (labels.includes(absent))
                bad.push(`type tier leaked '${absent}' (got ${labels.join(', ')})`);
        }

        // A6 pin: the brace form rides the names tier (`{` is a trigger
        // char — the extension fires the request on the keystroke)
        // through the MOUNTED pouch (the store world above — std rides
        // the dep lane, never the ambient wasm)
        const bsrc = 'use pouch::{\nfn main() -> nil {\n}\n';
        const buri = 'file:///ws/e2e-use-brace.rut';
        await std.analyze(buri, bsrc);
        const bitems = await std.complete(buri, 0, 'use pouch::{'.length, bsrc);
        const blabels = bitems.map((i) => i.label);
        if (!blabels.includes('Vec'))
            bad.push(`use brace form lacks pouch's 'Vec' (got ${blabels.join(', ')})`);
        smokes++;
    }

    // ---- member completion through a CONSTRUCTOR binding, with the
    // completion lane's doc sync + dot guard (the completion-correctness
    // phase): `let log = Logger("h")` types the binding from the
    // MOUNTED ink dep's class (the store world — std rides the dep
    // lane; the ctor-call inference), `log.` offers the impl methods —
    // and NEVER the bare tier's keyword/free-fn dump. The CURRENT text
    // rides the request (rut.complete's src argument → the doc-sync
    // complete), so the dot is fresh without the debounce. ----
    {
        const uri = 'file:///ws/e2e-ctor-completion.rut';
        // the member NAMES after each dot keep the doc parsing clean; the
        // request cursor sits right after the dot (prefix empty — the full
        // member list), exactly the keystroke moment in the editor
        const src = [
            'fn go() -> nil {',
            '    let log = Logger("h");',
            '    log.info;',
            '    someUnknown.x;',
            '}',
            '',
        ].join('\n');
        const a = await std.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `ctor-completion probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const [cline, cch] = posOf(src, 'log.'); // cursor right after the dot
        const items = await std.complete(uri, cline, cch + 4, src);
        const labels = items.map((i) => i.label);
        for (const want of ['new', 'debug', 'info', 'warn', 'error']) {
            if (!labels.includes(want))
                bad.push(`ctor binding completion lacks '${want}' (got ${labels.join(', ')})`);
        }
        if (labels.includes('abs'))
            bad.push(`ctor binding completion leaked the free fn 'abs' (got ${labels.join(', ')})`);
        // the dot guard: an unresolved receiver answers EMPTY, never the
        // keyword table
        const [uline, uch] = posOf(src, 'someUnknown.');
        const guarded = await std.complete(uri, uline, uch + 'someUnknown.'.length, src);
        if (guarded.length !== 0) {
            bad.push(
                `dot guard failed: 'someUnknown.' completed ${guarded.length} item(s): ${guarded
                    .slice(0, 8)
                    .map((i) => i.label)
                    .join(', ')}`,
            );
        }
        smokes++;
    }

    // ---- plan3 phase 1 (A7+A1): END-OF-BUFFER completion — the doc ends
    // EXACTLY at the incomplete statement, so the parser ends the
    // enclosing block's span at the EOF token and the cursor sits ON it.
    // Before the fn body's scope extension through the source end, every
    // binding of the block dropped and `list.` answered EMPTY at the most
    // common typing moment. The member tier answers through the real ABI
    // path now, and the locals tier (A1) offers the fn-local bindings
    // ahead of the keyword table. ----
    {
        const uri = 'file:///ws/e2e-eof.rut';
        const src = [
            'class List {',
            '    items: i32;',
            '}',
            'impl List {',
            '    fn count(self) -> i32 { return 0; }',
            '}',
            'fn go() -> nil {',
            '    let list = List.new();',
            '    list.',
        ].join('\n');
        // member completion at EOF: cursor right after the final dot, the
        // CURRENT text riding the request (the editor's exact call)
        const lastLine = src.slice(src.lastIndexOf('\n') + 1);
        const items = await rut.complete(uri, src.split('\n').length - 1, lastLine.length, src);
        const labels = items.map((i) => i.label);
        for (const want of ['items', 'count']) {
            if (!labels.includes(want))
                bad.push(`EOF member completion lacks '${want}' (got ${labels.join(', ')})`);
        }
        if (labels.includes('fn'))
            bad.push(`EOF member completion leaked the keyword table (got ${labels.join(', ')})`);

        // the locals shape at EOF: `che⏐` after `let mut checked …` — the
        // binding completes ahead of the keywords, riding LSP kind 6
        // (VARIABLE; the extension maps kind - 1, unchanged)
        const lsrc = 'fn go() -> nil {\n    let mut checked: str = " ";\n    che';
        const luri = 'file:///ws/e2e-eof-locals.rut';
        const litems = await rut.complete(
            luri,
            2,
            lsrc.slice(lsrc.lastIndexOf('\n') + 1).length,
            lsrc,
        );
        const checked = litems.find((i) => i.label === 'checked');
        if (!checked)
            bad.push(
                `EOF locals completion lacks 'checked' (got ${litems.map((i) => i.label).join(', ')})`,
            );
        else if (checked.kind !== 6)
            bad.push(`EOF local 'checked' kind ${checked.kind} != 6 (VARIABLE)`);
        const checkedAt = litems.findIndex((i) => i.label === 'checked');
        const letAt = litems.findIndex((i) => i.label === 'let');
        if (checkedAt >= 0 && letAt >= 0 && checkedAt > letAt) {
            bad.push(
                `EOF local 'checked' rides behind the keyword table (${checkedAt} > ${letAt})`,
            );
        }
        smokes++;
    }

    // ---- phase 1 (lsp-features): field DECL hover — hovering `item`
    // inside `struct Slot { item: ?Circle; }` answers with the field's
    // decl and its owning struct (the decl layer's token-recovered span) ----
    {
        const [line, ch] = posOnLine(fixtureSrc, /^\s*item: \?Circle;/, 'item');
        const h = await rut.hover(fixtureUri, line, ch);
        if (
            !h?.contents.value.includes('item: ?Circle') ||
            !h.contents.value.includes('in `Slot`')
        ) {
            bad.push(
                `field decl hover missed: ${h && JSON.stringify(h.contents.value.slice(0, 80))}`,
            );
        }
        smokes++;
    }

    // ---- phase 1: inferred-type IDENT hover — `let c = Circle.new(1.0)`
    // shows its inferred type at the use site (the display-side ruling) ----
    {
        const uri = 'file:///ws/e2e-inferred-ident.rut';
        const src = [
            'class Circle {',
            '    r: f64;',
            '}',
            'fn go() -> f64 {',
            '    let c = Circle.new(1.0);',
            '    return c.r;',
            '}',
            '',
        ].join('\n');
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `inferred-ident probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const [line, ch] = posOnLine(src, /return c\.r;/, 'c');
        const h = await rut.hover(uri, line, ch);
        if (
            !h?.contents.value.includes('let c: Circle') ||
            !h.contents.value.includes('type inferred')
        ) {
            bad.push(
                `inferred ident hover missed: ${h && JSON.stringify(h.contents.value.slice(0, 80))}`,
            );
        }
        smokes++;
    }

    // ---- phase 1: receiver inference through a FIELD READ — the
    // binding pass types `inner` from `wrap.c`, so the ident hover, the
    // member hover AND member completion all resolve through it ----
    {
        const uri = 'file:///ws/e2e-field-read-recv.rut';
        const src = [
            'class Circle {',
            '    r: f64;',
            '}',
            'struct Wrap {',
            '    c: Circle;',
            '}',
            'impl Circle {',
            '    fn grown(self, k: f64) -> Circle { return self; }',
            '}',
            'fn area(wrap: Wrap) -> f64 {',
            '    let inner = wrap.c;',
            '    let big = inner.grown(2.0);',
            '    return big.r;',
            '}',
            '',
        ].join('\n');
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `field-read probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        // the ident hover: the field-read initializer types the binding
        const [iline, ich] = posOnLine(src, /let big = inner\.grown/, 'inner');
        const ih = await rut.hover(uri, iline, ich);
        if (!ih?.contents.value.includes('let inner: Circle')) {
            bad.push(
                `field-read ident hover missed: ${ih && JSON.stringify(ih.contents.value.slice(0, 80))}`,
            );
        }
        // the member hover THROUGH the field-read receiver
        const mh = await rut.hover(uri, iline, ich + 7); // the `grown` after `inner.`
        if (!mh?.contents.value.includes('fn grown(self, k: f64) -> Circle')) {
            bad.push(
                `field-read receiver member hover missed: ${mh && JSON.stringify(mh.contents.value.slice(0, 80))}`,
            );
        }
        // the member hover through the CHAINED-CALL receiver (`big.r`)
        const [bline, bch] = posOnLine(src, /return big\.r;/, 'big.r');
        const bh = await rut.hover(uri, bline, bch + 4); // the `r` after `big.`
        if (!bh?.contents.value.includes('r: f64') || !bh.contents.value.includes('in `Circle`')) {
            bad.push(
                `chained receiver field hover missed: ${bh && JSON.stringify(bh.contents.value.slice(0, 80))}`,
            );
        }
        // member completion through the field-read receiver
        const items = await rut.complete(uri, iline, ich + 6, src); // right after `inner.`
        const labels = items.map((i) => i.label);
        for (const want of ['r', 'grown']) {
            if (!labels.includes(want))
                bad.push(`field-read completion lacks '${want}' (got ${labels.join(', ')})`);
        }
        smokes++;
    }

    // ---- phase 1: for-of loop-variable receiver — `[Point]` types the
    // iteration binding, its member hover resolves ----
    {
        const uri = 'file:///ws/e2e-for-of-recv.rut';
        const src = [
            'struct Point {',
            '    x: f64;',
            '    y: f64;',
            '}',
            'fn sum(points: [Point]) -> f64 {',
            '    let total = 0.0;',
            '    for (let p of points) {',
            '        total += p.x;',
            '    }',
            '    return total;',
            '}',
            '',
        ].join('\n');
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `for-of probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const [pline, pch] = posOnLine(src, /total \+= p\.x;/, 'p');
        const ph = await rut.hover(uri, pline, pch);
        if (
            !ph?.contents.value.includes('p: Point') ||
            !ph.contents.value.includes('loop variable')
        ) {
            bad.push(
                `for-of ident hover missed: ${ph && JSON.stringify(ph.contents.value.slice(0, 80))}`,
            );
        }
        const mh = await rut.hover(uri, pline, pch + 2); // the `x` after `p.`
        if (!mh?.contents.value.includes('x: f64') || !mh.contents.value.includes('in `Point`')) {
            bad.push(
                `for-of receiver member hover missed: ${mh && JSON.stringify(mh.contents.value.slice(0, 80))}`,
            );
        }
        smokes++;
    }

    // ---- phase 1 (M7): primitive type-token hover — `i32`/`bool` have
    // no surface decl; the static blurb (width, range) is the answer ----
    {
        const uri = 'file:///ws/e2e-prim-hover.rut';
        const src = 'fn pick(x: i32, ok: bool) -> i32 {\n    return x;\n}\n';
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `primitive probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const [line, ch] = posOnLine(src, /fn pick\(x: i32, ok: bool\)/, 'i32');
        const h = await rut.hover(uri, line, ch);
        if (
            !h?.contents.value.includes('signed 32-bit') ||
            !h.contents.value.includes('-2147483648 ..= 2147483647')
        ) {
            bad.push(
                `i32 primitive hover missed: ${h && JSON.stringify(h.contents.value.slice(0, 80))}`,
            );
        }
        const [bline, bch] = posOnLine(src, /fn pick\(x: i32, ok: bool\)/, 'bool');
        const bh = await rut.hover(uri, bline, bch);
        if (!bh?.contents.value.includes('true` / `false')) {
            bad.push(
                `bool primitive hover missed: ${bh && JSON.stringify(bh.contents.value.slice(0, 80))}`,
            );
        }
        smokes++;
    }

    // ---- phase 2 (lsp-features): definition round trips — request pos →
    // response span == the declaring ident EXACTLY. Three resolution
    // layers gated through the SHIPPED artifact: within-file (the binding
    // pass + decl layer), cross-file (the use graph), stdlib (the
    // store-mounted dep's true source file) ----

    // expected [line, char] of the ident at `at` inside `needle`'s line
    const identAt = (src, lineRe, needle) => {
        const [line, ch] = posOnLine(src, lineRe, needle);
        return [line, ch, needle.length];
    };
    // the response must carry a location with `want.uri` whose span is
    // the declaring ident EXACTLY — same start, same length
    const eqRange = (locs, want, what) => {
        const loc = locs.find((l) => l.uri === want.uri);
        if (!loc) {
            bad.push(`${what}: no location for ${want.uri} (got ${JSON.stringify(locs)})`);
            return;
        }
        const g = loc.range;
        const [wl, wc] = want.range;
        const wlen = want.len;
        if (
            g.start.line !== wl ||
            g.start.character !== wc ||
            g.end.line !== wl ||
            g.end.character !== wc + wlen
        ) {
            bad.push(
                `${what}: span ${JSON.stringify(g)} != declaring ident [${wl},${wc}..+${wlen}]`,
            );
        }
    };

    // WITHIN-FILE: the binding pass (local ident → its declaring let),
    // the decl layer (field read → the field's decl ident), and an impl
    // method call → the impl fn's name ident
    {
        const uri = 'file:///ws/e2e-def-within.rut';
        const src = [
            'class Circle {',
            '    r: f64;',
            '}',
            'struct Wrap {',
            '    c: Circle;',
            '}',
            'impl Circle {',
            '    fn area(self) -> f64 { return 3.14; }',
            '}',
            'fn go(wrap: Wrap) -> f64 {',
            '    let inner = wrap.c;',
            '    return inner.area() + inner.r;',
            '}',
            '',
        ].join('\n');
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `def-within probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        // `inner` at its use → the declaring ident in `let inner`
        let [dl, dc] = identAt(src, /return inner\.area\(\)/, 'inner');
        let want = { uri, range: identAt(src, /let inner = wrap\.c;/, 'inner'), len: 5 };
        let locs = await rut.definition(uri, dl, dc);
        eqRange(locs, want, 'within-file let binding');
        // field read `wrap.c` → the field's decl ident `c` in Wrap
        want = { uri, range: identAt(src, /^\s{4}c: Circle;/, 'c'), len: 1 };
        [dl, dc] = identAt(src, /let inner = wrap\.c;/, 'c');
        locs = await rut.definition(uri, dl, dc);
        eqRange(locs, want, 'within-file field read');
        // method call `inner.area()` → the impl fn's name ident
        want = { uri, range: identAt(src, /fn area\(self\)/, 'area'), len: 4 };
        [dl, dc] = identAt(src, /return inner\.area\(\)/, 'area');
        locs = await rut.definition(uri, dl, dc);
        eqRange(locs, want, 'within-file impl method');
        smokes++;
    }

    // CROSS-FILE: a `use gadgets::{ Widget }` doc jumps into the
    // exporting module (indexed via rut.add_def, pkg matched by its path
    // segment) — both the use-statement name and the usage sites
    {
        const libUri = 'file:///ws/gadgets/lib.rut';
        const libSrc = ['class Widget {', '    id: i32;', '}', ''].join('\n');
        await rut.addDef(libUri, libSrc);
        const uri = 'file:///ws/e2e-def-cross.rut';
        const src = [
            'use gadgets::{ Widget };',
            'fn main() -> nil {',
            '    let w = Widget.new();',
            '}',
            '',
        ].join('\n');
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `def-cross probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const want = {
            uri: libUri,
            range: identAt(libSrc, /^class Widget \{/, 'Widget'),
            len: 6,
        };
        // the use-statement name — the survey's use-graph edge
        let locs = await rut.definition(
            uri,
            ...identAt(src, /^use gadgets::\{ Widget \};/, 'Widget'),
        );
        eqRange(locs, want, 'cross-file use name');
        // and a usage site through the same edge
        locs = await rut.definition(uri, ...identAt(src, /let w = Widget\.new\(\);/, 'Widget'));
        eqRange(locs, want, 'cross-file usage');
        smokes++;
    }

    // STDLIB through the DEP LANE: `use pouch::{ Vec }` jumps into the
    // STORE-MOUNTED source — the response carries the walk's own file
    // spelling (the store's unpack tree, derived from the lock pin) and
    // the span of the declaring ident in the REAL pouch source the
    // fixture serves
    {
        const uri = 'file:///ws/e2e-def-std.rut';
        const src = [
            'use pouch::{ Vec };',
            'fn main() -> nil {',
            '    let v = Vec.new();',
            '}',
            '',
        ].join('\n');
        const a = await std.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `def-std probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const real = fs.readFileSync(path.join(REPO, 'rut', 'pouch', 'src', 'mod.rut'), 'utf8');
        const wantUri = `${POUCH_STORE_DIR}/unpack/src/mod.rut`;
        const want = { uri: wantUri, range: identAt(real, /^pub class Vec/, 'Vec'), len: 3 };
        let locs = await std.definition(uri, ...identAt(src, /^use pouch::\{ Vec \};/, 'Vec'));
        if (!locs.length || locs[0].uri !== wantUri) {
            bad.push(
                `std jump lost the store-mounted source path (got ${JSON.stringify(locs.map((l) => l.uri))})`,
            );
        } else {
            eqRange(locs, want, 'std use name');
        }
        locs = await std.definition(uri, ...identAt(src, /let v = Vec\.new\(\);/, 'Vec'));
        if (!locs.length || locs[0].uri !== wantUri) {
            bad.push(
                `std usage jump lost the store-mounted source path (got ${JSON.stringify(locs.map((l) => l.uri))})`,
            );
        } else {
            eqRange(locs, want, 'std usage');
        }
        smokes++;
    }

    // TYPE DEFINITION: an expression → its type's declaration (the cheap
    // shape the survey priced — binding types via the phase-1 inference)
    {
        const uri = 'file:///ws/e2e-typedef.rut';
        const src = [
            'class Circle {',
            '    r: f64;',
            '}',
            'fn go() -> f64 {',
            '    let c = Circle.new(1.0);',
            '    return c.r;',
            '}',
            '',
        ].join('\n');
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `typedef probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const want = { uri, range: identAt(src, /^class Circle \{/, 'Circle'), len: 6 };
        const locs = await rut.typeDefinition(uri, ...identAt(src, /return c\.r;/, 'c'));
        eqRange(locs, want, 'typeDefinition of a binding');
        smokes++;
    }

    // ---- phase 3 (lsp-features): INLAY hints through the SHIPPED
    // artifact. CORPUS ground truth: every hint's label is checked
    // against the source's own decl lines (an annotation elsewhere in the
    // file, the callee's ret, or the stdlib's true pouch decl — the
    // MOUNTED pouch, std being a dep like any other now). ----
    {
        const digestPath = path.join(REPO, 'examples', '02-digest', 'src', 'mod.rut');
        const digestSrc = fs.readFileSync(digestPath, 'utf8');
        const digestUri = `file://${digestPath}`;
        const a = await std.analyze(digestUri, digestSrc);
        if (a.diags.length !== 0) {
            bad.push(
                `digest inlay probe has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const all = await std.inlayHint(
            digestUri,
            { line: 0, character: 0 },
            { line: digestSrc.split('\n').length, character: 0 },
        );
        if (!all.length) bad.push('digest inlay: no hints at all');
        // a TYPE hint hangs at the END of the binding ident (needle = the
        // ident, so the hint position is needle-end); a PARAMETER hint sits
        // at the arg's first byte (needle = the arg's first token)
        const hintAt = (src, lineRe, needle, wantLabel, wantKind, gt, what) => {
            const [line, ch] = posOnLine(src, lineRe, needle);
            const at = wantKind === 1 ? ch + needle.length : ch;
            const h = all.find((x) => x.position.line === line && x.position.character === at);
            if (!h) {
                bad.push(
                    `${what}: no hint at ${line}:${at} (have ${all
                        .filter((x) => x.position.line === line)
                        .map((x) => `${x.position.character}:${x.label}`)
                        .join(', ')})`,
                );
                return;
            }
            if (h.label !== wantLabel) {
                bad.push(
                    `${what}: label ${JSON.stringify(h.label)} != ${JSON.stringify(wantLabel)}`,
                );
            }
            if (h.kind !== wantKind) {
                bad.push(`${what}: kind ${h.kind} != ${wantKind}`);
            }
            if (gt && !gt.test(src)) {
                bad.push(`${what}: ground-truth line missing from the corpus file`);
            }
        };
        // `let src = "0123456789abcdef";` → str — GT: the file's own
        // `fn hex_val(c: str)` param annotation
        hintAt(
            digestSrc,
            /let src = "0123456789abcdef";/,
            'src',
            ': str',
            1,
            /^fn hex_val\(c: str\) -> i32 \{/m,
            'digest str let',
        );
        // `let d = hex_digits();` → Vec<str> — GT: the file's ANNOTATED
        // `let d: Vec<str> = Vec::new();` three lines up (the corpus spells
        // the paths grammar's `::` since fe7b5f94)
        hintAt(
            digestSrc,
            /let d = hex_digits\(\);/,
            'd',
            ': Vec<str>',
            1,
            /^ {4}let d: Vec<str> = Vec::new\(\);/m,
            'digest vec let',
        );
        // `let v = hex_val(f"{c}");` → i32 — GT: the callee's `-> i32` ret
        hintAt(
            digestSrc,
            /let v = hex_val\(f/,
            'v',
            ': i32',
            1,
            /^fn hex_val\(c: str\) -> i32 \{/m,
            'digest call ret let',
        );
        // for-c counter → i32 — GT: RFC 0007 §1 (the suffixless literal
        // default; the same law the file's `let mut i = 0;` hints ride)
        // (std-v10 era: the mut-law sweep spells the header `let mut i`)
        hintAt(
            digestSrc,
            /for \(let mut i = 0; i < n; i \+= 3\) \{/,
            'i',
            ': i32',
            1,
            /^ {4}let mut i = 0;/m,
            'digest for-c counter',
        );
        // PARAMETER hint at the free call: `hex_val(f"{c}")` → `c:` —
        // GT: the callee's own param name in its decl line
        hintAt(
            digestSrc,
            /let v = hex_val\(f"\{c\}"\);/,
            'f',
            'c:',
            2,
            /^fn hex_val\(c: str\) -> i32 \{/m,
            'digest free-call param',
        );
        // PARAMETER hint through the CROSS-FILE method: `d.push(f"{c}")` →
        // `v:` — GT: the TRUE stdlib decl `pub fn push(mut self, v: T)`
        const pouchSrc = fs.readFileSync(path.join(REPO, 'rut', 'pouch', 'src', 'mod.rut'), 'utf8');
        hintAt(digestSrc, /d\.push\(f"\{c\}"\);/, 'f', 'v:', 2, null, 'digest push param');
        if (!/pub fn push\(mut self, v: T\) -> nil/.test(pouchSrc)) {
            bad.push('digest push param: ground-truth push decl missing from pouch src/mod.rut');
        }
        // the hover-consistency law, gated through the artifact: the
        // hint's type text IS what hover shows for the same binding
        {
            const [line, ch] = posOnLine(digestSrc, /let d = hex_digits\(\);/, 'd');
            const h = await std.hover(digestUri, line, ch);
            const hint = all.find(
                (x) => x.position.line === line && x.position.character === ch + 1,
            );
            if (!h || !hint || !h.contents.value.includes(`let d: ${hint.label.slice(2)}`)) {
                bad.push(
                    `digest hover/hint consistency lost: hover=${h && JSON.stringify(h.contents.value.slice(0, 60))} hint=${hint && JSON.stringify(hint.label)}`,
                );
            }
        }
        smokes++;
    }

    // ---- phase 3: for-of corpus ground truth — the element of an
    // ANNOTATED Vec<i32> types the loop variable, in a second corpus
    // file (demo/src/examples/closures-generics/src/mod.rut) ----
    {
        const cgPath = path.join(
            REPO,
            'demo',
            'src',
            'examples',
            'closures-generics',
            'src',
            'mod.rut',
        );
        const cgSrc = fs.readFileSync(cgPath, 'utf8');
        const cgUri = `file://${cgPath}`;
        const a = await rut.analyze(cgUri, cgSrc);
        if (a.diags.length !== 0) {
            bad.push(
                `closures-generics inlay probe has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const all = await rut.inlayHint(
            cgUri,
            { line: 0, character: 0 },
            { line: 9999, character: 0 },
        );
        const hintAt = (lineRe, needle, wantLabel, what) => {
            const [line, ch] = posOnLine(cgSrc, lineRe, needle);
            const h = all.find(
                (x) => x.position.line === line && x.position.character === ch + needle.length,
            );
            if (!h) {
                bad.push(`${what}: no hint at ${line}:${ch + needle.length}`);
                return;
            }
            if (h.label !== wantLabel || h.kind !== 1) {
                bad.push(
                    `${what}: got ${JSON.stringify(h.label)} kind ${h.kind}, wanted ${JSON.stringify(wantLabel)} kind 1`,
                );
            }
        };
        // `for (let x of xs)` → i32 — GT: the iterable's own annotation
        // `fn sum(xs: Vec<i32>)`: the element of Vec<i32> IS i32
        hintAt(/for \(let x of xs\) \{/, 'x', ': i32', 'for-of elem');
        // `let mut total = 0;` → i32 — the RFC 0007 literal default, GT:
        // the file's annotated `a: i32` closure params
        hintAt(/let mut total = 0;/, 'total', ': i32', 'literal default');
        if (!/fn sum\(xs: Vec<i32>\) -> i32 \{/.test(cgSrc)) {
            bad.push('for-of elem: ground-truth Vec<i32> annotation missing');
        }
        smokes++;
    }

    // ---- phase 3: the synthetic laws — annotated lets are NEVER
    // restated, param hints skip `self`, the range filter narrows, and
    // the tooltip is the hover markdown ----
    {
        const uri = 'file:///ws/e2e-inlay.rut';
        const src = [
            'class Circle {',
            '    r: f64;',
            '}',
            'impl Circle {',
            '    fn grown(self, k: f64) -> Circle { return self; }',
            '}',
            'fn go() -> Circle {',
            '    let c = Circle.new(1.0);',
            '    let d: Circle = Circle.new(1.0);',
            '    return c.grown(2.0);',
            '}',
            '',
        ].join('\n');
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `inlay probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const all = await rut.inlayHint(uri, { line: 0, character: 0 }, { line: 99, character: 0 });
        const labels = all.map((h) => `${h.position.line}:${h.label}(k${h.kind})`);
        // exactly two: the unannotated `c` and the exact-arity grown arg —
        // the ANNOTATED `d` is restated by nothing
        if (labels.join(', ') !== '7:: Circle(k1), 9:k:(k2)') {
            bad.push(`inlay synthetic hints drifted: [${labels.join(', ')}]`);
        }
        const tip = all[0]?.tooltip?.value;
        if (!tip?.includes('let c: Circle') || !tip.includes('type inferred')) {
            bad.push(`inlay tooltip is not the hover markdown: ${JSON.stringify(tip)}`);
        }
        // the range filter: only line 7's hint stays inside [7, 8)
        const narrow = await rut.inlayHint(
            uri,
            { line: 7, character: 0 },
            { line: 8, character: 0 },
        );
        if (!(narrow.length === 1 && narrow[0].label === ': Circle')) {
            bad.push(`inlay range filter leaked: ${JSON.stringify(narrow.map((h) => h.label))}`);
        }
        smokes++;
    }

    // ---- phase 4 (lsp-features): REFERENCES round trips — a decl ->
    // the EXACT expected use-position set. Shadow-aware across scopes,
    // and cross-file through the use graph's reverse edges in BOTH
    // directions (lib-decl query finds the importer's sites; doc-side
    // query finds the same set through the doc's own use graph). ----

    // the byte offset of a reference — locations must carry (line, char)
    const refAt = (src, needle, occurrence = 1) => {
        const [line, ch] = posOf(src, needle, occurrence);
        return line * 100000 + ch;
    };
    const refOffsets = (_src, locs, uri) =>
        locs
            .filter((l) => l.uri === uri)
            .map((l) => l.range.start.line * 100000 + l.range.start.character)
            .sort((a, b) => a - b);

    // WITHIN-FILE + SHADOW-AWARE: the outer v answers with its if-cond
    // use + final return; the inner shadow answers with exactly its own
    // return — neither leaks across the scope boundary
    {
        const uri = 'file:///ws/e2e-refs-shadow.rut';
        const src = [
            'fn go() -> i32 {',
            '    let v = 1;',
            '    if (v > 0) {',
            '        let v = 2;',
            '        return v;',
            '    }',
            '    return v;',
            '}',
            '',
        ].join('\n');
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `refs-shadow probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        // the OUTER decl
        let [dl, dc] = posOf(src, 'let v = 1;', 1);
        let locs = await rut.references(uri, dl, dc + 4, false);
        const want = [refAt(src, 'v > 0'), refAt(src, 'return v;', 2) + 7].sort((x, y) => x - y);
        const got = refOffsets(src, locs, uri);
        if (JSON.stringify(got) !== JSON.stringify(want)) {
            bad.push(`outer v refs drifted: ${JSON.stringify(got)} != ${JSON.stringify(want)}`);
        }
        // the INNER decl: exactly its own return
        [dl, dc] = posOf(src, 'let v = 2;', 1);
        locs = await rut.references(uri, dl, dc + 4, false);
        if (!(locs.length === 1 && locs[0].range.start.line === 4)) {
            bad.push(`inner v refs leaked: ${JSON.stringify(locs)}`);
        }
        // includeDeclaration appends the declaring ident
        locs = await rut.references(uri, dl, dc + 4, true);
        if (
            !(
                locs.length === 2 &&
                locs.some((l) => l.range.start.line === 3 && l.range.start.character === dc + 4)
            )
        ) {
            bad.push(`includeDeclaration lost the decl ident: ${JSON.stringify(locs)}`);
        }
        smokes++;
    }

    // CROSS-FILE, both directions: the lib is indexed via rut.add_def
    // (pkg matched by its path segment); the importing doc's use name +
    // usage are the EXACT reference set of the lib's decl — queried from
    // the doc side AND from the lib-decl side
    {
        const libUri = 'file:///ws/gadgets/lib.rut';
        const libSrc = 'class Widget {\n    id: i32;\n}\n';
        const mainUri = 'file:///ws/e2e-refs-cross.rut';
        const mainSrc =
            'use gadgets::{ Widget };\nfn main() -> nil {\n    let w = Widget.new();\n}\n';
        await rut.addDef(libUri, libSrc);
        await rut.addDef(mainUri, mainSrc); // the extension indexes every workspace file
        const a = await rut.analyze(mainUri, mainSrc);
        if (a.diags.length !== 0) {
            bad.push(
                `refs-cross probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const want = [refAt(mainSrc, 'Widget }'), refAt(mainSrc, 'Widget.new')].sort(
            (x, y) => x - y,
        );
        // from the importing doc's usage site
        let [dl, dc] = posOf(mainSrc, 'Widget.new', 1);
        let locs = await rut.references(mainUri, dl, dc, false);
        const got = refOffsets(mainSrc, locs, mainUri);
        if (JSON.stringify(got) !== JSON.stringify(want)) {
            bad.push(
                `cross-file refs (doc side) drifted: ${JSON.stringify(got)} != ${JSON.stringify(want)}`,
            );
        }
        // from the lib's DECL ident: the same two sites, carrying the doc's uri
        const a2 = await rut.analyze(libUri, libSrc);
        if (a2.diags.length !== 0) {
            bad.push(
                `refs-cross lib doc has ${a2.diags.length} diagnostic(s): ${JSON.stringify(a2.diags[0])}`,
            );
        }
        [dl, dc] = posOf(libSrc, 'class Widget', 1);
        locs = await rut.references(libUri, dl, dc + 6, false);
        if (!(locs.length === 2 && locs.every((l) => l.uri === mainUri))) {
            bad.push(`cross-file refs (lib side) drifted: ${JSON.stringify(locs)}`);
        }
        smokes++;
    }

    // ---- phase 4: SIGNATURE HELP — the verbatim signature, the active
    // parameter from the comma/paren depth (first/middle/nested), the
    // method receiver through its type head, and a mismatch -> no help ----
    {
        const uri = 'file:///ws/e2e-sighelp.rut';
        const src = [
            'fn inner(a: i32) -> i32 {',
            '    return a;',
            '}',
            'fn outer(b: i32, c: i32) -> i32 {',
            '    return b + c;',
            '}',
            'fn hex_val(c: str, k: i32) -> i32 {',
            '    return k;',
            '}',
            'class Circle {',
            '    r: f64;',
            '}',
            'impl Circle {',
            '    fn grown(self, k: f64) -> Circle { return self; }',
            '}',
            'fn arity(a: i32, b: i32) -> i32 {',
            '    return a;',
            '}',
            'fn main() -> i32 {',
            '    let x = outer(inner(1), 2);',
            '    let h = hex_val("a", x);',
            '    let bad: i32 = arity(1, 2, 3);',
            '    let c = Circle.new(1.0);',
            '    let g = c.grown(2.0);',
            '    return x + h + bad + g.r as i32;',
            '}',
            '',
        ].join('\n');
        const a = await rut.analyze(uri, src);
        if (a.diags.length !== 0) {
            bad.push(
                `sighelp probe doc has ${a.diags.length} diagnostic(s): ${JSON.stringify(a.diags[0])}`,
            );
        }
        const wantHelp = async (pos, wantLabel, wantActive, what) => {
            const h = await rut.signatureHelp(uri, pos[0], pos[1]);
            if (!h) {
                bad.push(
                    `${what}: no help (wanted ${JSON.stringify(wantLabel)} slot ${wantActive})`,
                );
                return;
            }
            const sig = h.signatures[0];
            if (sig.label !== wantLabel) {
                bad.push(
                    `${what}: label ${JSON.stringify(sig.label)} != ${JSON.stringify(wantLabel)}`,
                );
            }
            if (h.activeParameter !== wantActive) {
                bad.push(`${what}: activeParameter ${h.activeParameter} != ${wantActive}`);
            }
        };
        // FIRST arg of a nested call: the innermost unclosed paren wins
        await wantHelp(
            posOf(src, 'inner(1)', 1).map((v, i) => (i === 1 ? v + 6 : v)),
            'fn inner(a: i32) -> i32',
            0,
            'nested first arg',
        );
        // MIDDLE arg (after the nested call closed): the outer's second slot
        await wantHelp(
            posOf(src, ', 2);', 1).map((v, i) => (i === 1 ? v + 2 : v)),
            'fn outer(b: i32, c: i32) -> i32',
            1,
            'outer second arg',
        );
        // the outer's FIRST arg (the inner call itself): slot 0
        {
            const [ol, oc] = posOf(src, 'outer(inner', 1);
            await wantHelp([ol, oc + 6], 'fn outer(b: i32, c: i32) -> i32', 0, 'outer first arg');
        }
        // free call: first arg then second
        await wantHelp(
            posOf(src, '"a"', 1),
            'fn hex_val(c: str, k: i32) -> i32',
            0,
            'free call first arg',
        );
        await wantHelp(
            posOf(src, ', x);', 1).map((v, i) => (i === 1 ? v + 2 : v)),
            'fn hex_val(c: str, k: i32) -> i32',
            1,
            'free call second arg',
        );
        // METHOD call through a typed receiver: self never surfaces
        {
            const [ml, mc] = posOf(src, 'grown(2.0)', 1);
            const h = await rut.signatureHelp(uri, ml, mc + 6);
            if (
                h?.signatures[0].label !== 'fn grown(self, k: f64) -> Circle' ||
                h.activeParameter !== 0
            ) {
                bad.push(`method receiver help drifted: ${JSON.stringify(h)}`);
            } else {
                const labels = h.signatures[0].parameters.map((p) => p.label);
                if (JSON.stringify(labels) !== JSON.stringify(['k: f64'])) {
                    bad.push(`method param pieces drifted: ${JSON.stringify(labels)}`);
                }
            }
        }
        // a mismatch shows NO help, never wrong help
        {
            const [bl, bc] = posOf(src, '2, 3);', 1);
            const h = await rut.signatureHelp(uri, bl, bc + 5);
            if (h !== null) {
                bad.push(`arity mismatch must show no help: ${JSON.stringify(h)}`);
            }
        }
        smokes++;
    }

    // ---- the dedicated postfix-removal diagnostic (M2's acceptance;
    // the RFC sweep reworded it — the message names the fix, not a
    // proposal number) ----
    {
        const b = await rut.analyze('file:///ws/e2e-postfix.rut', 'fn f(p: i32?) -> nil {\n}\n');
        if (!(b.diags.length === 1 && b.diags[0].message.includes('postfix spelling'))) {
            bad.push(
                `postfix T? must give the ONE dedicated postfix-removal diag, got: ${JSON.stringify(b.diags)}`,
            );
        }
        const g = await rut.analyze('file:///ws/e2e-prefix.rut', 'fn g(p: ?i32) -> nil {\n}\n');
        if (g.diags.length !== 0) {
            bad.push(`?i32 must analyze clean (M1), got: ${JSON.stringify(g.diags)}`);
        }
        smokes++;
    }

    // ---- phase 5 (manifest-deps): the DEP ABI — parse_manifest /
    // add_def / add_def_mod / set_env + resolve_deps — through the
    // SHIPPED artifact's binding. Separate instance, loaded over the
    // SAME std store world (std rides the dep lane: ink mounts here,
    // so the doc-sync lane's Logger assertions read the real walk),
    // plus whatever this lane indexes — the binding lane above keeps
    // its own instance unpolluted for the corpus walk. ----
    {
        // a FRESH binding instance: dropping the require cache re-executes
        // the bundle, so the wasm-bindgen module instantiates AGAIN from the
        // same shipped bytes — this lane's defs are the prelude, the
        // mounted std world, and exactly what it indexes; the corpus lane
        // above keeps its instance unpolluted
        delete require.cache[BINDING_PATH];
        const depBinding = require(BINDING_PATH);
        const dep = await depBinding.RutLsp.load(fs.readFileSync(WASM_PATH), makeStdEnv());
        const depWalk = await dep.resolveDeps(['/ws/stdproj']);
        assert.ok(
            !depWalk.error && depWalk.missing.length === 0,
            `the dep-lane std store mounts: ${JSON.stringify(depWalk)}`,
        );

        // the manifest: the repo's own path+url fixture (03-plugin) — the
        // same bytes rut-lsp's dep_table unit test pins
        const manifestPath = path.join(REPO, 'examples', '03-plugin', 'plugin', 'rut.jsonc');
        const manifest = fs.readFileSync(manifestPath, 'utf8');
        const table = await dep.parseManifest(manifest);
        assert.strictEqual(table.name, 'plugin', `manifest name: ${JSON.stringify(table)}`);
        // the body rides the src/mod.rut convention (the entry keys are
        // repealed — the root module is a file under the `src/` shell,
        // never a manifest key)
        assert.deepStrictEqual(
            table.entries,
            [{ path: 'src/mod.rut', mode: 'Impl' }],
            `entry modes: ${JSON.stringify(table.entries)}`,
        );
        const row = (name) => table.deps.find((r) => r.name === name);
        assert.deepStrictEqual(
            row('server'),
            {
                name: 'server',
                source: { kind: 'path', dir: '../server' },
                optional: false,
                lib: null,
            },
            `path row: ${JSON.stringify(row('server'))}`,
        );
        const pouchRow = row('pouch');
        assert.strictEqual(pouchRow.source.kind, 'url');
        // the expected URL comes from the FIXTURE's own bytes, never a
        // hardcoded tag here — a std-tag sweep edits rut.jsonc and this
        // gate follows it, so the two can never drift again (a pinned
        // std-v8 URL once failed this lane BEFORE the corpus lock ran and
        // masked the alignment verdict entirely)
        const pouchUrl = manifest.match(/"pouch"\s*:\s*\{\s*"url"\s*:\s*"([^"]+)"/)[1];
        assert.ok(pouchUrl, 'the fixture manifest carries a pouch url dep');
        assert.strictEqual(pouchRow.source.url, pouchUrl);
        // the manifest row mirrors the fixture bytes — and since the
        // rut.lock migration (66c8e6a8) a manifest url row is
        // { "url": ... } only, so the parsed sha256 rides as null
        assert.strictEqual(pouchRow.source.sha256, null);
        // the pin itself lives in the companion rut.lock (the pin is law
        // at the mount door): hex64, riding the SAME url, and equal to the
        // sha256 of the COMMITTED bundle bytes the url names (the tag
        // segment of the url IS the dist dir) — all derived from fixture
        // bytes, so a std-tag sweep edits the fixtures and this gate
        // follows; no constant here can drift
        const lock = fs.readFileSync(path.join(path.dirname(manifestPath), 'rut.lock'), 'utf8');
        const lockPouch = lock.match(/\{[^{}]*"pkg"\s*:\s*"pouch"[^{}]*\}/)[0];
        assert.strictEqual(
            lockPouch.match(/"url"\s*:\s*"([^"]+)"/)[1],
            pouchUrl,
            'the lock row rides the manifest url',
        );
        const pin = lockPouch.match(/"sha256"\s*:\s*"([0-9a-f]{64})"/)[1];
        assert.ok(pin, 'the lock pins the pouch bundle (hex64)');
        const pouchBundle = fs.readFileSync(
            path.join(REPO, 'dist', pouchUrl.match(/rut@([^/]+)\//)[1], 'pouch.rutbundle'),
        );
        assert.strictEqual(
            crypto.createHash('sha256').update(pouchBundle).digest('hex'),
            pin,
            'the lock pin matches the committed bundle bytes',
        );
        // the cache path is computed RUST-side — sha256 of the URL string
        // (crates/rut-lsp-wasm/src/deps.rs cache_path); the binding never
        // hashes, so the gate recomputes the SAME digest over the fixture
        // URL and the row must equal it — derived, never a pinned constant
        assert.strictEqual(
            pouchRow.source.cache_path,
            `.rut/cache/${crypto.createHash('sha256').update(pouchUrl).digest('hex')}.rutbundle`,
        );
        assert.deepStrictEqual(table.dev_deps, []);
        // the peer row set is gone from the wire (the retired `[peer-deps]`
        // table's client-visible removal) — the envelope spells no
        // `peer_deps` row at all
        assert.strictEqual('peer_deps' in table, false, 'the envelope carries no peer_deps row');
        smokes++;

        // entry.type indexes in Decl mode (the envelope spells the variant)
        const hostManifest =
            '{ "name": "host", "type": "host", "entry": { "type": "./surface.d.rut" } }';
        const host = await dep.parseManifest(hostManifest);
        assert.deepStrictEqual(
            host.entries,
            [{ path: 'src/surface.d.rut', mode: 'Decl' }],
            `decl entry: ${JSON.stringify(host.entries)}`,
        );
        // a malformed manifest is the ERROR envelope — the host's hint
        // (named `manifestErr`, NOT `bad` — that name is the lane's
        // error collector and a shadow here silences every later push)
        const manifestErr = await dep.parseManifest('{"name": six}');
        assert.ok(
            manifestErr.error?.startsWith('line 1: '),
            `error envelope: ${JSON.stringify(manifestErr)}`,
        );
        smokes++;

        // ---- the named-dep use-path tier through the WALK (the old
        // push lane is gone from the ABI): a PATH dep read through the
        // injected env — resolveDeps mounts it, then the `use <name>::`
        // tier lists the dep's public surface and `use ⏐` the name ----
        {
            const gfiles = new Map([
                [
                    '/ws/gproj/rut.jsonc',
                    '{\n  "name": "gproj",\n  "deps": { "gadgets": { "path": "../gadgets" } }\n}\n',
                ],
                ['/ws/gproj/src/mod.rut', 'fn main() -> nil {\n}\n'],
                ['/ws/gadgets/rut.jsonc', '{\n  "name": "gadgets"\n}\n'],
                [
                    '/ws/gadgets/src/mod.rut',
                    'pub class Widget {\n    id: i32;\n}\npub fn tag() -> i32 {\n    return 1;\n}\n',
                ],
            ]);
            const norm = (rp) => rp.replace(/\/[^/]+\/\.\./g, '').replace('//', '/');
            const genc = new TextEncoder();
            const wrut = await RutLsp.load(fs.readFileSync(WASM_PATH), {
                read: (rp) => {
                    const v = gfiles.get(norm(rp));
                    return v === undefined ? null : genc.encode(v);
                },
                readDir: () => null,
                isDir: () => false,
                notify: () => {},
            });
            const wout = await wrut.resolveDeps(['/ws/gproj']);
            assert.ok(
                !wout.error && wout.missing.length === 0,
                `walk answer: ${JSON.stringify(wout)}`,
            );
            const depDoc = 'use gadgets::\nfn main() -> nil {\n}\n';
            const depUri = 'file:///ws/e2e-dep.rut';
            await wrut.analyze(depUri, depDoc);
            const namesTier = await wrut.complete(depUri, 0, depDoc.indexOf('::') + 2, depDoc);
            const depLabels = namesTier.map((i) => i.label);
            for (const want of ['Widget', 'tag']) {
                if (!depLabels.includes(want))
                    bad.push(
                        `walked dep 'gadgets' lacks '${want}' in use completion (got ${depLabels.join(', ')})`,
                    );
            }
            const modTier = await wrut.complete(depUri, 0, 4, depDoc);
            if (!modTier.map((i) => i.label).includes('gadgets'))
                bad.push('the module-name tier lacks the walked dep');
        }
        smokes++;

        {
            const wsRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'rut-e2e-walk-'));
            try {
                // ---- the path-dep lane: a mod-tree dep's REAL files — the
                // middle segment lands on the `pub mod layout;` edge, the leaf
                // in the child file ----
                const toolsDir = path.join(wsRoot, 'tools2');
                const rootSrc = 'pub mod layout;\npub fn tag() -> i32 { return 1; }\n';
                const childSrc =
                    'pub struct Column {\n    w: i32;\n}\npub fn mk() -> Column { return Column { w: 1 }; }\nmod grid;\n';
                fs.mkdirSync(path.join(toolsDir, 'src', 'layout'), { recursive: true });
                fs.writeFileSync(path.join(toolsDir, 'src', 'mod.rut'), rootSrc);
                fs.writeFileSync(path.join(toolsDir, 'src', 'layout', 'mod.rut'), childSrc);
                let ts = await rut.addDefMod(
                    `file://${path.join(toolsDir, 'src', 'mod.rut')}`,
                    'tools2',
                    '',
                    rootSrc,
                );
                assert.ok(!ts.error, `tools2 root indexes: ${JSON.stringify(ts)}`);
                ts = await rut.addDefMod(
                    `file://${path.join(toolsDir, 'src', 'layout', 'mod.rut')}`,
                    'tools2',
                    'layout',
                    childSrc,
                );
                assert.ok(!ts.error, `tools2 child indexes: ${JSON.stringify(ts)}`);

                const mdoc =
                    'use tools2::layout::{ Column };\nfn main() -> nil {\n    let c: Column = Column { w: 1 };\n}\n';
                const muri = 'file:///ws/e2e-dep-mods.rut';
                const ma = await rut.analyze(muri, mdoc);
                assert.strictEqual(
                    ma.diags.length,
                    0,
                    `dep-mods probe doc: ${JSON.stringify(ma.diags)}`,
                );
                // the MIDDLE segment: the mounting edge in the root file
                const midLocs = await rut.definition(muri, 0, 'use tools2::'.length);
                assert.strictEqual(
                    midLocs.length,
                    1,
                    `middle segment targets: ${JSON.stringify(midLocs)}`,
                );
                assert.ok(
                    midLocs[0].uri.endsWith('/tools2/src/mod.rut'),
                    `middle segment lands in the root: ${midLocs[0].uri}`,
                );
                eqRange(
                    midLocs,
                    {
                        uri: midLocs[0].uri,
                        range: identAt(rootSrc, /^pub mod layout;/, 'layout'),
                        len: 6,
                    },
                    'path-dep middle segment',
                );
                // the LEAF: the decl in the child file
                const leafLocs = await rut.definition(muri, ...posOf(mdoc, 'Column'));
                assert.strictEqual(leafLocs.length, 1, `leaf targets: ${JSON.stringify(leafLocs)}`);
                assert.ok(
                    leafLocs[0].uri.endsWith('/tools2/src/layout/mod.rut'),
                    `leaf lands in the child: ${leafLocs[0].uri}`,
                );
                eqRange(
                    leafLocs,
                    {
                        uri: leafLocs[0].uri,
                        range: identAt(childSrc, /^pub struct Column/, 'Column'),
                        len: 6,
                    },
                    'path-dep leaf',
                );
                // and the usage site rides the same edge into the child
                const usageLocs = await rut.definition(muri, ...posOf(mdoc, 'Column { w: 1 }'));
                assert.ok(
                    usageLocs.some((l) => l.uri.endsWith('/tools2/src/layout/mod.rut')),
                    `usage into the child: ${JSON.stringify(usageLocs)}`,
                );

                // ---- the editor-agreement lane: the doc IS a package file —
                // the mod-edge click opens the mounted child at its file top,
                // and a file-form child clicks to its REAL file (never a
                // re-derived grid/mod.rut) ----
                const gridSrc = 'pub fn dig() -> i32 { return 1; }\n';
                fs.writeFileSync(path.join(toolsDir, 'src', 'layout', 'grid.rut'), gridSrc);
                ts = await rut.addDefMod(
                    `file://${path.join(toolsDir, 'src', 'layout', 'grid.rut')}`,
                    'tools2',
                    'layout/grid',
                    gridSrc,
                );
                assert.ok(!ts.error, `tools2 grid.rut indexes: ${JSON.stringify(ts)}`);

                const rootUri = `file://${path.join(toolsDir, 'src', 'mod.rut')}`;
                await rut.analyze(rootUri, rootSrc); // the doc's stamp rides its indexed twin
                const edgeLocs = await rut.definition(rootUri, 0, 'pub mod '.length);
                assert.strictEqual(
                    edgeLocs.length,
                    1,
                    `edge click targets: ${JSON.stringify(edgeLocs)}`,
                );
                assert.ok(
                    edgeLocs[0].uri.endsWith('/tools2/src/layout/mod.rut'),
                    `edge click lands in the child: ${edgeLocs[0].uri}`,
                );
                assert.deepStrictEqual(
                    edgeLocs[0].range,
                    { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } },
                    `edge click opens the file top: ${JSON.stringify(edgeLocs[0].range)}`,
                );

                const layoutUri = `file://${path.join(toolsDir, 'src', 'layout', 'mod.rut')}`;
                await rut.analyze(layoutUri, childSrc);
                const gridLocs = await rut.definition(layoutUri, ...posOf(childSrc, 'grid'));
                assert.strictEqual(
                    gridLocs.length,
                    1,
                    `file-form child targets: ${JSON.stringify(gridLocs)}`,
                );
                assert.ok(
                    gridLocs[0].uri.endsWith('/tools2/src/layout/grid.rut'),
                    `file-form child lands in grid.rut: ${gridLocs[0].uri}`,
                );
                assert.deepStrictEqual(
                    gridLocs[0].range,
                    { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } },
                    `file-form child opens the file top: ${JSON.stringify(gridLocs[0].range)}`,
                );
                smokes++;
            } finally {
                fs.rmSync(wsRoot, { recursive: true, force: true });
            }
        }

        // ---- file modules (the mod walk): rut_add_def_mod stamps the
        // tree — the root's `pub mod layout;` rides back as the mount
        // walk's next hop, `use tools::layout::` completes the walked
        // module's exports (never the private edge), and the use-path
        // segment hover names the mounted file. A fresh package name: the
        // lane's earlier flat `gadgets` index stays the transitional shape
        // it was indexed as. ----
        const mRoot = 'pub mod layout;\nmod secret;\npub fn tag() -> i32 { return 1; }\n';
        const rootOut = await dep.addDefMod('file:///ws/tools/src/mod.rut', 'tools', '', mRoot);
        assert.deepStrictEqual(
            rootOut.decls,
            [
                { name: 'layout', vis: 'pub' },
                { name: 'secret', vis: 'mod' },
            ],
            `root mod decls: ${JSON.stringify(rootOut.decls)}`,
        );
        const mChild =
            'pub struct Column {\n    w: i32;\n}\npub fn mk() -> Column { return Column { w: 1 }; }\n';
        const childOut = await dep.addDefMod(
            'file:///ws/tools/src/layout/mod.rut',
            'tools',
            'layout',
            mChild,
        );
        assert.deepStrictEqual(
            childOut.decls,
            [],
            `leaf mod decls: ${JSON.stringify(childOut.decls)}`,
        );

        const modDoc =
            'use tools::layout::{ Column };\nfn main() -> nil {\n    let c: Column = Column { w: 1 };\n}\n';
        const modUri = 'file:///ws/e2e-mods.rut';
        await dep.analyze(modUri, modDoc);
        // the walked tier: the resolved module's exports
        const walkTier = await dep.complete(modUri, 0, 'use tools::layout::'.length, modDoc);
        const walkLabels = walkTier.map((i) => i.label);
        for (const want of ['Column', 'mk']) {
            assert.ok(
                walkLabels.includes(want),
                `mod walk lacks '${want}' (got ${walkLabels.join(', ')})`,
            );
        }
        // the root tier: the pub mod child offers, the private edge and
        // the child's members never leak
        const rootTier = await dep.complete(modUri, 0, 'use tools::'.length, modDoc);
        const rootLabels = rootTier.map((i) => i.label);
        assert.ok(
            rootLabels.includes('layout'),
            `root tier mod edges: ${JSON.stringify(rootLabels)}`,
        );
        assert.ok(
            !rootLabels.includes('secret'),
            `a bare \`mod\` edge never crosses: ${JSON.stringify(rootLabels)}`,
        );
        assert.ok(
            !rootLabels.includes('Column'),
            `child members stay under their path: ${JSON.stringify(rootLabels)}`,
        );
        // the segment hover resolves the mounted module
        const segHover = await dep.hover(modUri, 0, 'use tools::layout::'.length - 3);
        assert.ok(
            segHover?.contents.value.includes('layout/mod.rut'),
            `segment hover missed: ${segHover && JSON.stringify(segHover.contents.value.slice(0, 80))}`,
        );
        smokes++;

        // ---- the completion lane's doc-sync ABI (complete with src): the
        // CURRENT text is stored BEFORE the tiers run, so a stale stored doc
        // (the tail still short the dot — the analyze debounce's window)
        // cannot mask the member tier, and the dot guard holds through the
        // raw entry (an unresolved receiver answers [], never the dump). ----
        assert.strictEqual(
            typeof dep.complete,
            'function',
            'doc-sync completion missing from the binding — rebuild (npm run build:wasm)',
        );
        const syncUri = 'file:///ws/e2e-sync.rut';
        const staleDoc = 'fn go() -> nil {\n    let log = Logger("h");\n    log\n}\n';
        const freshDoc = 'fn go() -> nil {\n    let log = Logger("h");\n    log.\n}\n';
        await dep.analyze(syncUri, staleDoc);
        const synced = await dep.complete(syncUri, 2, 8, freshDoc);
        const syncedLabels = synced.map((i) => i.label);
        for (const want of ['new', 'debug', 'info', 'warn', 'error']) {
            assert.ok(
                syncedLabels.includes(want),
                `doc-sync completion lacks '${want}' over the stale stored doc (got ${syncedLabels.join(', ')})`,
            );
        }
        assert.ok(
            !syncedLabels.includes('abs'),
            `doc-sync completion leaked the host fn 'abs' (got ${syncedLabels.join(', ')})`,
        );
        // the dot guard through the raw entry: `probe().` — [] exactly
        const guardDoc = 'fn go() -> nil {\n    let x = probe().\n}\n';
        const guarded = await dep.complete(syncUri, 1, 20, guardDoc);
        assert.deepStrictEqual(
            guarded,
            [],
            `dot guard through the raw entry: ${JSON.stringify(guarded)}`,
        );
        smokes++;

        // ---- plan3 phase 4 (B1+B2): SEMANTIC DIAGNOSTICS — the check lane.
        // A doc whose package is STAMPED (rut_add_def_mod named it) compiles
        // with its dep closure: the checker's type errors surface with the
        // right file and range — the bool→str assignment mismatch AND the
        // missing record field (the diagnosis pair). The dep rides the chain
        // (mounted via rut_add_def_mod); an UNSTAMPED doc with the same text
        // stays parse-only clean — today's behavior, the corpus lock above
        // depends on it. ----
        {
            const chkDepSrc =
                'pub struct Task {\n    id: u32;\n    title: str;\n    completed: bool;\n}\n';
            const chkDep = await dep.addDefMod(
                'file:///ws/chk/dep/src/mod.rut',
                'chkdep',
                '',
                chkDepSrc,
            );
            assert.ok(Array.isArray(chkDep.decls), `chkdep stamps: ${JSON.stringify(chkDep)}`);
            const chkAppSrc = [
                'use chkdep::{ Task };',
                'fn main() -> nil {',
                '    let mut checked: str = " ";',
                '    checked = true;',
                '    let t = Task { title: "x", completed: false };',
                '}',
                '',
            ].join('\n');
            const chkAppUri = 'file:///ws/chk/app/src/mod.rut';
            const chkApp = await dep.addDefMod(chkAppUri, 'chkapp', '', chkAppSrc);
            assert.ok(Array.isArray(chkApp.decls), `chkapp stamps: ${JSON.stringify(chkApp)}`);
            const chkMerged = await dep.analyze(chkAppUri, chkAppSrc);
            const chkDiags = chkMerged.diags || [];
            const mismatch = chkDiags.find((d) =>
                (d.message || '').includes('assignment type mismatch'),
            );
            const missingField = chkDiags.find((d) => (d.message || '').includes('id` is missing'));
            if (!mismatch) {
                bad.push(
                    `check lane: no assignment-mismatch diag (got ${JSON.stringify(chkDiags.map((d) => d.message))})`,
                );
            } else {
                if (mismatch.source !== 'rut')
                    bad.push(`check lane: mismatch source ${mismatch.source} != rut`);
                if (mismatch.range.start.line !== 3) {
                    bad.push(
                        `check lane: mismatch diag on line ${mismatch.range.start.line} != 3 (${JSON.stringify(mismatch.range)})`,
                    );
                }
            }
            if (!missingField) {
                bad.push(
                    `check lane: no missing-field diag (got ${JSON.stringify(chkDiags.map((d) => d.message))})`,
                );
            } else if (missingField.range.start.line !== 4) {
                bad.push(
                    `check lane: missing-field diag on line ${missingField.range.start.line} != 4`,
                );
            }
            // the parse-only fallback: the SAME text at an unstamped uri
            const chkPlain = await dep.analyze('file:///ws/chk/plain.rut', chkAppSrc);
            if ((chkPlain.diags || []).length !== 0) {
                bad.push(
                    `unstamped doc must stay parse-only clean: ${JSON.stringify(chkPlain.diags)}`,
                );
            }
            smokes++;
        }
    }

    // ---- dep-walk-into-crate phase 2: the injected READ-ONLY env —
    // load(bytes, env) rides set_env on the fresh core (the host
    // capability the walk's next phase reads). This phase only STORES
    // it, so the gate proves the capability LANDS (load resolves, the
    // core still answers, nothing called back into the env yet) and
    // that omitting env boots exactly as before (the demo path). ----
    {
        const enc = new TextEncoder();
        const files = new Map(
            Object.entries({
                'deps/kit@abc1230123456789/mod.rut':
                    'pub fn kit_tag() -> i32 {\n    return 1;\n}\n',
                'deps/kit@abc1230123456789/src/util.rut':
                    'pub fn deep() -> i32 {\n    return 2;\n}\n',
            }),
        );
        const dirEntries = new Map([
            ['deps/kit@abc1230123456789', ['mod.rut', 'src']],
            ['deps/kit@abc1230123456789/src', ['util.rut']],
        ]);
        const reads = [];
        const notes = [];
        const fakeEnv = {
            read(p) {
                reads.push(p);
                const src = files.get(p);
                return src === undefined ? null : enc.encode(src);
            },
            readDir(p) {
                return dirEntries.get(p) ?? null;
            },
            isDir(p) {
                return dirEntries.has(p);
            },
            notify(message, isError) {
                notes.push({ message, isError });
            },
        };
        const envRut = await RutLsp.load(fs.readFileSync(WASM_PATH), fakeEnv);
        // set_env resolved (load returned) and the env-carrying core
        // still answers: a clean analyze + the prelude intact
        const envUri = 'file:///ws/e2e-env.rut';
        const envSrc = 'fn main() -> nil {\n}\n';
        const ea = await envRut.analyze(envUri, envSrc);
        assert.strictEqual(
            ea.diags.length,
            0,
            `env-injected probe doc: ${JSON.stringify(ea.diags)}`,
        );
        assert.ok(
            (await envRut.legend()).includes('keyword'),
            'the env-carrying core still boots the prelude',
        );
        // the zero-behavior law: no entry read the env this phase (the
        // walk is the next one) and the hint lane stayed quiet
        assert.deepStrictEqual(reads, [], 'phase 2 stores the env, nothing reads it');
        assert.deepStrictEqual(notes, [], 'the hint lane stays quiet this phase');
        // omitting env (the demo path): boots and answers exactly as before
        const plainRut = await RutLsp.load(fs.readFileSync(WASM_PATH));
        const pa = await plainRut.analyze(envUri, envSrc);
        assert.strictEqual(pa.diags.length, 0, `env-less boot: ${JSON.stringify(pa.diags)}`);
        smokes++;
    }

    // ---- dep-walk-into-crate phase 3: the READ-ONLY WALK through the
    // injected env — a fake env (pure JS maps) over a FIXTURE STORE
    // (meta.json + the unpack/ tree, keyed at the address rut-driver
    // derives; the wasm target's `$RUT_HOME` fallback is `.rut`). A
    // store hit mounts through the ordinary dir lanes — definitions
    // land in real store paths, the embedded dep group included — and
    // a missing dep answers the missing row plus exactly ONE hint
    // (never fetched; the walk has no write lane at all). ----
    {
        const enc = new TextEncoder();
        const PIN = 'abcdef0123456789'.repeat(4);
        const GONE_PIN = '0123456789abcdef'.repeat(4);
        const KIT_URL = 'https://cdn.jsdelivr.net/gh/example/rut@std-v1/dist/std/kit.zip';
        const GONE_URL = 'https://cdn.jsdelivr.net/gh/example/rut@std-v1/dist/std/gone.zip';
        const KIT_DIR = `.rut/deps/kit@${PIN.slice(0, 16)}`;
        const files = new Map(
            Object.entries({
                // the project root: a lock pinning both urls, a
                // manifest depending on both, the root module
                '/ws/walkproj/rut.lock': JSON.stringify(
                    {
                        format: 'rutlock',
                        format_version: 1,
                        deps: [
                            { pkg: 'kit', url: KIT_URL, sha256: PIN },
                            { pkg: 'gone', url: GONE_URL, sha256: GONE_PIN },
                        ],
                    },
                    null,
                    2,
                ),
                '/ws/walkproj/rut.jsonc': JSON.stringify({
                    name: 'walkapp',
                    deps: { kit: { url: KIT_URL }, gone: { url: GONE_URL } },
                }),
                '/ws/walkproj/src/mod.rut': 'fn main() -> nil {\n}\n',
                // the FIXTURE STORE: one pinned dep (meta vouches the
                // lock pin) with a mod child and an embedded group
                [`${KIT_DIR}/meta.json`]: JSON.stringify({
                    name: 'kit',
                    url: KIT_URL,
                    sha256: PIN,
                    format: 'zip',
                    format_version: 1,
                }),
                [`${KIT_DIR}/unpack/rut.jsonc`]: JSON.stringify({
                    name: 'kit',
                    deps: { kitcore: { path: 'deps/kitcore@feed0123456789abc' } },
                }),
                [`${KIT_DIR}/unpack/src/mod.rut`]:
                    'pub mod extra;\npub class KitBox {\n    n: i32;\n}\n',
                [`${KIT_DIR}/unpack/src/extra/mod.rut`]:
                    'pub fn kit_extra() -> i32 {\n    return 7;\n}\n',
                [`${KIT_DIR}/unpack/deps/kitcore@feed0123456789abc/rut.jsonc`]: JSON.stringify({
                    name: 'kitcore',
                }),
                [`${KIT_DIR}/unpack/deps/kitcore@feed0123456789abc/src/mod.rut`]:
                    'pub fn core_tag() -> i32 {\n    return 9;\n}\n',
            }),
        );
        const reads = [];
        const notes = [];
        const fakeEnv = {
            read(p) {
                reads.push(p);
                const src = files.get(p);
                return src === undefined ? null : enc.encode(src);
            },
            readDir(p) {
                const names = [...files.keys()]
                    .filter((k) => k.startsWith(`${p}/`))
                    .map((k) => k.slice(p.length + 1).split('/')[0]);
                return names.length ? [...new Set(names)] : null;
            },
            isDir(p) {
                return [...files.keys()].some((k) => k.startsWith(`${p}/`));
            },
            notify(message, isError) {
                notes.push({ message, isError });
            },
        };
        const rut = await RutLsp.load(fs.readFileSync(WASM_PATH), fakeEnv);
        const out = await rut.resolveDeps(['/ws/walkproj']);
        assert.ok(!out.error, `resolve answer: ${JSON.stringify(out)}`);
        // kit mounted from the store; gone refused with its row
        assert.strictEqual(out.missing.length, 1, JSON.stringify(out));
        assert.strictEqual(out.missing[0].url, GONE_URL);
        assert.strictEqual(out.missing[0].sha256, GONE_PIN);
        assert.ok(out.missing[0].cache_path.startsWith('.rut/cache/'));
        assert.deepStrictEqual(
            Object.keys(out.missing[0]).sort(),
            ['cache_path', 'sha256', 'url'],
            'the missing row carries exactly the declared keys',
        );
        // exactly ONE hint — the missing dep's `rut install` recipe —
        // and the re-resolve never repeats it (the once-per-reason
        // gate lives in the core) while the row keeps answering
        const out2 = await rut.resolveDeps(['/ws/walkproj']);
        assert.strictEqual(out2.missing.length, 1, JSON.stringify(out2));
        assert.strictEqual(notes.length, 1, JSON.stringify(notes));
        assert.ok(notes[0].message.includes('rut install'), notes[0].message);
        assert.ok(notes[0].message.includes('gone'), notes[0].message);
        assert.strictEqual(notes[0].isError, true);
        // the walk read ONLY through the env, only the two worlds it
        // was given (the project root and the derived store address)
        assert.ok(reads.length > 0, 'the walk read through the env');
        assert.ok(
            reads.every((p) => p.startsWith('/ws/walkproj/') || p.startsWith('.rut/deps/')),
            `unexpected read: ${reads.find((p) => !p.startsWith('/ws/walkproj/') && !p.startsWith('.rut/deps/'))}`,
        );
        // definitions land in REAL store paths: the pkg root, the mod
        // child at its own path, and the embedded group through the
        // pkg's own path row
        const doc =
            'use kit::{ KitBox };\n' +
            'use kit::extra::{ kit_extra };\n' +
            'use kitcore::{ core_tag };\n' +
            'fn main() -> nil {\n}\n';
        const wuri = 'file:///ws/e2e-walk.rut';
        const wa = await rut.analyze(wuri, doc);
        assert.strictEqual(wa.diags.length, 0, JSON.stringify(wa.diags));
        const expectStoreLeaf = async (needle, tail) => {
            const locs = await rut.definition(wuri, ...posOf(doc, needle));
            assert.strictEqual(
                locs.length,
                1,
                `${needle}: ${JSON.stringify(locs)} (want ${KIT_DIR}/${tail})`,
            );
            assert.ok(locs[0].uri === `${KIT_DIR}/${tail}`, `${needle} landed at ${locs[0].uri}`);
        };
        await expectStoreLeaf('KitBox', 'unpack/src/mod.rut');
        await expectStoreLeaf('kit_extra', 'unpack/src/extra/mod.rut');
        await expectStoreLeaf('core_tag', 'unpack/deps/kitcore@feed0123456789abc/src/mod.rut');
        smokes++;

        // ---- the stir lane through the shipped wasm: the ACTOR-owned
        // scheduling — the env's one machinery capability is a dumb
        // clock (`wakeAfter` = plain setTimeout), the loop defers +
        // coalesces + wakes ITSELF through its own inbox; `stir`'s
        // promise resolves with the WalkOut once the deferred walk runs
        // (REAL Node timers). The depEngine shell this block once drove
        // is deleted — the actor owns the when now. ----
        {
            const clockRut = await RutLsp.load(fs.readFileSync(WASM_PATH), {
                ...fakeEnv,
                wakeAfter: (ms, wake) => setTimeout(wake, ms),
            });
            const t0 = Date.now();
            const out = await clockRut.stir(['/ws/walkproj']);
            const waited = Date.now() - t0;
            assert.ok(waited >= 150, `the stir defers under the window (took ${waited}ms)`);
            // the WalkOut envelope over the SAME world the eager door
            // walked: the gone dep refused, the store dep MOUNTED
            assert.strictEqual(out.missing.length, 1, JSON.stringify(out));
            assert.strictEqual(out.missing[0].url, GONE_URL);
            const sdoc = 'use kit::{ KitBox };\nfn main() -> nil {\n}\n';
            const suri = 'file:///ws/e2e-stir.rut';
            const sa = await clockRut.analyze(suri, sdoc);
            assert.strictEqual(sa.diags.length, 0, JSON.stringify(sa.diags));
            const slocs = await clockRut.definition(suri, ...posOf(sdoc, 'KitBox'));
            assert.strictEqual(slocs.length, 1, JSON.stringify(slocs));
            assert.ok(slocs[0].uri === `${KIT_DIR}/unpack/src/mod.rut`, slocs[0].uri);
            // coalescing through the ABI: the first stir of a burst is
            // superseded (`null` — merged), the LAST carries the world
            const p1 = clockRut.stir(['/ws/walkproj']);
            const p2 = clockRut.stir(['/ws/walkproj']);
            assert.strictEqual(await p1, null, 'the superseded stir merges');
            const out2 = await p2;
            assert.strictEqual(out2.missing.length, 1, JSON.stringify(out2));
            smokes++;
        }
    }

    // ---- the ABI wire shape (the .d.ts contract's runtime half) ----
    // The declarations live in the wasm package
    // (crates/rut-lsp-wasm/pkg/index.d.ts, assembled from the crate's js/).
    // `tsc --noEmit` checks the hosts; only this block can catch a
    // Rust-side rename, because the wire is data. Envelopes the crate OWNS
    // get exact key sets; `ls_types` payloads allow the declared fields
    // only — an unknown key means the .d.ts drifted from the wire.
    {
        const core = await RutLsp.load(fs.readFileSync(WASM_PATH));
        const keysOf = (o) => Object.keys(o).sort();
        const eqKeys = (o, want, what) =>
            assert.deepStrictEqual(
                keysOf(o),
                [...want].sort(),
                `${what} keys: ${JSON.stringify(keysOf(o))}`,
            );
        const declaredOnly = (o, allowed, what) => {
            const extra = keysOf(o).filter((k) => !allowed.includes(k));
            assert.deepStrictEqual(
                extra,
                [],
                `${what} carries keys the .d.ts does not declare: ${extra}`,
            );
        };

        // analyze: the crate's own envelope, exactly
        const src =
            'class Widget {\n    id: i32;\n}\nfn go(w: Widget) -> i32 {\n    return w.id;\n}\n';
        const uri = 'file:///ws/wire-shape.rut';
        const analysis = await core.analyze(uri, src);
        eqKeys(analysis, ['diags', 'symbols', 'tokens'], 'analysis');
        eqKeys(analysis.tokens, ['data'], 'analysis.tokens');

        // a symbol / a diagnostic / a hover — ls_types values, declared subset
        const sym = analysis.symbols[0];
        assert.ok(sym, 'the doc has a symbol');
        declaredOnly(
            sym,
            ['name', 'detail', 'kind', 'range', 'selectionRange', 'children', 'tags', 'deprecated'],
            'symbol',
        );
        eqKeys(sym.range, ['start', 'end'], 'symbol.range');
        eqKeys(sym.range.start, ['line', 'character'], 'position');

        const broken = await core.analyze('file:///ws/wire-diag.rut', 'fn go( {\n}\n');
        assert.ok(broken.diags.length > 0, 'the broken doc carries a parse diag');
        declaredOnly(
            broken.diags[0],
            [
                'range',
                'severity',
                'source',
                'message',
                'relatedInformation',
                'code',
                'codeDescription',
                'tags',
                'data',
            ],
            'diagnostic',
        );

        const hover = await core.hover(uri, 0, 6);
        if (hover) {
            declaredOnly(hover, ['contents', 'range'], 'hover');
            eqKeys(hover.contents, ['kind', 'value'], 'hover.contents');
        }

        // completion items and definition targets
        const items = await core.complete(uri, 4, 13, src);
        assert.ok(items.length > 0, 'member completion answers');
        declaredOnly(
            items[0],
            ['label', 'kind', 'detail', 'documentation', 'sortText', 'additionalTextEdits'],
            'completion item',
        );
        const defs = await core.definition(uri, 0, 6);
        assert.ok(defs.length > 0, 'the class name defines');
        eqKeys(defs[0], ['range', 'uri'], 'location');
        eqKeys(defs[0].range, ['start', 'end'], 'location.range');

        // inlay hints + signature help
        for (const h of await core.inlayHint(
            uri,
            { line: 0, character: 0 },
            { line: 5, character: 0 },
        )) {
            declaredOnly(
                h,
                [
                    'position',
                    'label',
                    'kind',
                    'tooltip',
                    'paddingLeft',
                    'paddingRight',
                    'textEdits',
                    'command',
                ],
                'inlay hint',
            );
        }
        const call =
            'fn add(a: i32, b: i32) -> i32 {\n    return a + b;\n}\nfn go() -> i32 {\n    return add(1, 2);\n}\n';
        const curi = 'file:///ws/wire-sig.rut';
        await core.analyze(curi, call);
        const sh = await core.signatureHelp(curi, 4, 16);
        if (sh) {
            declaredOnly(
                sh,
                ['signatures', 'activeSignature', 'activeParameter'],
                'signature help',
            );
            declaredOnly(
                sh.signatures[0],
                ['label', 'documentation', 'parameters', 'activeParameter'],
                'signature',
            );
            eqKeys(sh.signatures[0].parameters[0], ['label'], 'parameter');
        }

        // the dep face — the crate's own envelopes, exactly
        const manifestPath = path.join(REPO, 'examples', '03-plugin', 'plugin', 'rut.jsonc');
        const table = await core.parseManifest(fs.readFileSync(manifestPath, 'utf8'));
        assert.ok(!table.error, `manifest parse: ${JSON.stringify(table)}`);
        eqKeys(table, ['name', 'entries', 'consts', 'deps', 'dev_deps'], 'dep table');
        eqKeys(table.entries[0], ['path', 'mode'], 'entry source');
        eqKeys(table.deps[0], ['name', 'source', 'optional', 'lib'], 'dep row');
        eqKeys(
            table.deps.find((r) => r.source.kind === 'path').source,
            ['kind', 'dir'],
            'path dep source',
        );
        const urlRow = table.deps.find((r) => r.source.kind === 'url');
        if (urlRow)
            eqKeys(urlRow.source, ['kind', 'url', 'sha256', 'cache_path'], 'url dep source');
        eqKeys(await core.parseManifest('{'), ['error'], 'manifest error envelope');

        // the dep walk's envelope — the env-less core above answers the
        // loud error envelope; an env-carrying core answers WalkOut's
        // exact key set (a roots-everywhere-miss world: no rows, but
        // the shape is the shape)
        eqKeys(await core.resolveDeps(['/ws/none']), ['error'], 'resolve no-env');
        const walkCore = await RutLsp.load(fs.readFileSync(WASM_PATH), {
            read: () => null,
            readDir: () => null,
            isDir: () => false,
            notify: () => {},
        });
        eqKeys(await walkCore.resolveDeps(['/ws/none']), ['missing'], 'walk out');
        smokes++;
    }

    if (bad.length) {
        console.error(
            `e2e-wasm: ${bad.length} violation(s):\n  ${bad.slice(0, 40).join('\n  ')}${bad.length > 40 ? `\n  … (+${bad.length - 40} more)` : ''}`,
        );
        process.exit(1);
    }
    console.log(
        `e2e-wasm: PASS — ${files.length} corpus files, 0 false diagnostics, ${symbols} symbols, ` +
            `${smokes} smoke assertions (legend/fixture/?T hover/primitives/member-nullable/` +
            `ambient-core-honest-none/` +
            `std-store-mount+mounted-bare-import/` +
            `ctor-completion+dot-guard/postfix-removal/` +
            `field-decl-hover/inferred-ident/field-read-receiver/for-of-receiver/primitive-hover/` +
            `def-within/def-cross/def-std-store-mounted/typeDefinition/` +
            `inlay-corpus-ground-truth/inlay-for-of-corpus/inlay-synthetic-laws/` +
            `refs-shadow/refs-cross/sighelp/` +
            `manifest-dep-table/manifest-decl+error/walked-dep-use-tier/` +
            `path-dep-segments/` +
            `eof-member+locals/` +
            `complete-doc-sync/check-lane-mismatch+missing-field/` +
            `env-injected+env-less/` +
            `dep-walk-store-mount+missing-row+once-hint/` +
            `stir-clock+coalesce/wire-shape) ` +
            `through bin/rut-lsp.wasm`,
    );
}

main().catch((e) => {
    console.error('e2e-wasm: FAILED —', e);
    process.exit(1);
});
