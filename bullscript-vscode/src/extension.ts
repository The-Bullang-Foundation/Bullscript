import { workspace, ExtensionContext, window } from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

export function activate(_context: ExtensionContext) {
    const config = workspace.getConfiguration('bullscript');
    const serverPath = config.get<string>('serverPath', 'bullscript');

    const serverOptions: ServerOptions = {
        run:   { command: serverPath, args: ['lsp'] },
        debug: { command: serverPath, args: ['lsp'] },
    };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'bullscript' }],
        synchronize: {
            fileEvents: workspace.createFileSystemWatcher('**/*.busc'),
        },
    };

    client = new LanguageClient(
        'bullscript',
        'BullScript Language Server',
        serverOptions,
        clientOptions,
    );

    client.start().catch(err => {
        window.showErrorMessage(
            `BullScript language server failed to start: ${err.message}\n` +
            `Make sure 'bullscript' is on your PATH, or set bullscript.serverPath in settings.`
        );
    });
}

export function deactivate(): Thenable<void> | undefined {
    return client?.stop();
}
