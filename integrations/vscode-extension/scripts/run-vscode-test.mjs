// run test/runTest.js inside a real VS Code Extension Host and propagate
// the result — the headless(ish) "does it actually work in VS Code" gate.
// npm scripts run with cwd = the extension folder (demo/ uses the same).
import { spawn, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const root = process.cwd();

function findCodeExe() {
  if (process.platform === 'win32') {
    const local = join(process.env.LOCALAPPDATA ?? '', 'Programs', 'Microsoft VS Code', 'Code.exe');
    if (existsSync(local)) return local;
    const where = spawnSync('where', ['code'], { encoding: 'utf8' });
    if (where.status === 0) {
      // .../bin/code.cmd -> Code.exe sits two levels up
      const shim = where.stdout.trim().split(/\r?\n/)[0];
      const guess = resolve(shim, '..', '..', 'Code.exe');
      if (existsSync(guess)) return guess;
    }
  }
  if (process.platform === 'darwin') {
    const mac = '/Applications/Visual Studio Code.app/Contents/MacOS/Electron';
    if (existsSync(mac)) return mac;
  }
  return 'code'; // linux / PATH fallback
}

const exe = findCodeExe();
const args = [
  `--extensionDevelopmentPath=${join(root)}`,
  `--extensionTestsPath=${join(root, 'test', 'runTest.js')}`,
  `--user-data-dir=${join(tmpdir(), 'rut-vscode-test-user')}`,
  `--extensions-dir=${join(tmpdir(), 'rut-vscode-test-ext')}`,
  '--disable-gpu',
  '--disable-workspace-trust',
  '--skip-welcome',
  join(root), // workspace = the extension folder
];

console.log(`launching: ${exe} (test host)`);
const child = spawn(exe, args, { stdio: 'inherit' });
child.on('close', (code) => {
  console.log(code === 0 ? '\nVS Code extension tests: PASS' : `\nVS Code extension tests: FAIL (exit ${code})`);
  process.exit(code ?? 1);
});
