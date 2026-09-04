// rut-vscode — launches rut-lsp over stdio. Without the server binary the
// extension still activates the TextMate grammar (basic highlighting) and
// points the user at `npm run build:server`.

import * as fs from 'node:fs';
import * as path from 'node:path';
import { ExtensionContext, OutputChannel, window, workspace } from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

function serverFileName(): string {
  return process.platform === 'win32' ? 'rut-lsp.exe' : 'rut-lsp';
}

function resolveServerPath(): string {
  const config = workspace.getConfiguration('rut');
  const configured = config.get<string>('serverPath') ?? '';
  if (configured !== '') {
    return configured;
  }
  // out/extension.js -> ../bin/rut-lsp(.exe) (dropped there by build:server)
  const bundled = path.join(__dirname, '..', 'bin', serverFileName());
  return fs.existsSync(bundled) ? bundled : '';
}

export function activate(_context: ExtensionContext): void {
  const serverPath = resolveServerPath();
  if (serverPath === '') {
    window.showInformationMessage(
      'rut-lsp not found — run `npm run build:server` in integrations/vscode-extension ' +
        '(or set `rut.serverPath`) for semantic highlighting, diagnostics and symbols. ' +
        'Basic grammar highlighting stays on.'
    );
    return;
  }

  const serverOptions: ServerOptions = {
    command: serverPath,
    transport: TransportKind.stdio,
  };
  const outputChannel: OutputChannel = window.createOutputChannel('rut');
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ language: 'rut' }],
    outputChannel,
    traceOutputChannel: outputChannel,
  };
  client = new LanguageClient('rut', 'rut language server', serverOptions, clientOptions);
  client.start();
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
