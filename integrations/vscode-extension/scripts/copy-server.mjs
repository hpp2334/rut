// copy the cargo-built rut-lsp binary into bin/ (platform-aware)
import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';

const exe = process.platform === 'win32' ? 'rut-lsp.exe' : 'rut-lsp';
const src = join('..', '..', 'target', 'release', exe);
if (!existsSync(src)) {
  console.error(`server binary not found: ${src} — run \`cargo build -p rut-lsp --release\` first`);
  process.exit(1);
}
mkdirSync('bin', { recursive: true });
copyFileSync(src, join('bin', exe));
console.log(`copied ${src} -> bin/${exe}`);
