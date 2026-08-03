import * as vscode from 'vscode';
import { FreecodeAssistantViewProvider } from './provider';
import { FreecodeClient } from './client';

export function activate(context: vscode.ExtensionContext) {
    console.log('FreeCode extension is now active!');

    // Initialize View Provider
    const provider = new FreecodeAssistantViewProvider(context.extensionUri);
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider(
            FreecodeAssistantViewProvider.viewType,
            provider
        )
    );

    // Common client for quick command shortcuts
    const client = new FreecodeClient('127.0.0.1:50051');

    // Register Ping Command
    let pingCommand = vscode.commands.registerCommand('freecode.ping', async () => {
        try {
            vscode.window.showInformationMessage('Pinging FreeCode Daemon...');
            const res = await client.ping();
            vscode.window.showInformationMessage(`FreeCode Daemon Connection OK! Version: ${res.version}, Status: ${res.status}`);
        } catch (err: any) {
            vscode.window.showErrorMessage(`Failed to connect to FreeCode Daemon: ${err.message || err}`);
        }
    });

    // Register Dispatch Intent Command
    let askCommand = vscode.commands.registerCommand('freecode.ask', async () => {
        const prompt = await vscode.window.showInputBox({
            prompt: 'Enter intent instructions for FreeCode AI',
            placeHolder: 'e.g. Refactor auth module to use secure password hashing'
        });

        if (!prompt) {
            return;
        }

        const mode = await vscode.window.showQuickPick(['hitl', 'auto', 'chat'], {
            placeHolder: 'Select execution mode',
            title: 'FreeCode Mode'
        });

        if (!mode) {
            return;
        }

        try {
            vscode.window.showInformationMessage(`Dispatching FreeCode intent in '${mode}' mode...`);
            const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || '.';
            let finalMessage = '';
            let finalSessionId = '';
            client.dispatchIntent(
                prompt,
                mode,
                workspaceFolder,
                '',
                '',
                '',
                (res) => {
                    if (res.status === 'status') {
                        finalMessage = res.message;
                        finalSessionId = res.session_id;
                    }
                },
                () => {
                    vscode.window.showInformationMessage(`Daemon Dispatch Success: ${finalMessage.substring(0, 100)}... (Session: ${finalSessionId})`);
                },
                (err: any) => {
                    vscode.window.showErrorMessage(`Failed to dispatch intent: ${err.message || err}`);
                }
            );
        } catch (err: any) {
            vscode.window.showErrorMessage(`Failed to dispatch intent: ${err.message || err}`);
        }
    });

    context.subscriptions.push(pingCommand);
    context.subscriptions.push(askCommand);
}

export function deactivate() {}
