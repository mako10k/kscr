const vscode = require('vscode');
const path = require('path');

const {
  LanguageClient,
  TransportKind,
} = require('vscode-languageclient/node');

let client;

function resolveServerCommand(serverPathSetting) {
  if (serverPathSetting && typeof serverPathSetting === 'string' && serverPathSetting.trim() !== '') {
    const p = serverPathSetting.trim();
    // If the user provides a relative path, resolve it relative to the first workspace folder.
    const ws = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
    if (ws && !path.isAbsolute(p) && (p.includes('/') || p.includes('\\'))) {
      return path.join(ws.uri.fsPath, p);
    }
    return p;
  }
  // Default: rely on PATH.
  return 'kscr-lsp';
}

async function startClient(context) {
  const cfg = vscode.workspace.getConfiguration('kscr');
  const enabled = cfg.get('lsp.enabled', true);
  if (!enabled) {
    return;
  }

  const serverCommand = resolveServerCommand(cfg.get('lsp.serverPath'));

  const serverOptions = {
    command: serverCommand,
    args: [],
    transport: TransportKind.stdio,
  };

  const clientOptions = {
    documentSelector: [{ scheme: 'file', language: 'kscr' }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher('**/*.ks'),
    },
  };

  client = new LanguageClient('kscr', 'kscr Language Server', serverOptions, clientOptions);
  try {
    const started = client.start();
    context.subscriptions.push(started);
    if (started && typeof started.then === 'function') {
      await started;
    }
    if (typeof client.onReady === 'function') {
      await client.onReady();
    }
  } catch (e) {
    client = undefined;
    vscode.window.showErrorMessage(
      `kscr-lsp failed to start: ${e}. Set kscr.lsp.serverPath to your kscr-lsp binary path (see docs/LSP_Quick_Start.md).`
    );
  }
}

async function stopClient() {
  if (!client) return;
  const c = client;
  client = undefined;
  await c.stop();
}

/** @param {vscode.ExtensionContext} context */
function activate(context) {
  context.subscriptions.push(
    vscode.commands.registerCommand('kscr.lsp.restart', async () => {
      await stopClient();
      await startClient(context);
      vscode.window.showInformationMessage('kscr: Language Server restarted.');
    })
  );

  startClient(context);
}

function deactivate() {
  return stopClient();
}

module.exports = {
  activate,
  deactivate,
};
