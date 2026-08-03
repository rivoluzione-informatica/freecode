import * as vscode from 'vscode';
import { FreecodeClient } from './client';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';
import * as cp from 'child_process';
import * as crypto from 'crypto';
import { getWebviewCss } from './webview/style';
import { getWebviewJs } from './webview/client';
import { getWebviewHtml } from './webview/markup';

/** Single source of truth for the "daemon not reachable" guidance (reused by the ping/test path
 *  and the dispatch error path) so a transport failure never leaks a raw grpc-js string. */
const DAEMON_OFFLINE_MSG =
    'FreeCode daemon offline (127.0.0.1:50051). Start it — `cargo run -p freecode-daemon` or the freecode launchd service — then retry.';

/** Classify a dispatch error so connection failures (grpc-js code 14 / ECONNREFUSED, whose raw
 *  message includes an empty "Resolution note:") get the friendly guidance instead of the raw text. */
function classifyDaemonError(err: any): { kind: 'offline' | 'daemon'; message: string } {
    const raw = String(err?.message ?? err ?? '').trim();
    if (err?.code === 14 || /UNAVAILABLE|ECONNREFUSED|No connection established/i.test(raw)) {
        return { kind: 'offline', message: DAEMON_OFFLINE_MSG };
    }
    return { kind: 'daemon', message: `FreeCode daemon error: ${raw}` };
}

export class FreecodeAssistantViewProvider implements vscode.WebviewViewProvider {
    public static readonly viewType = 'freecode.assistantView';
    private _view?: vscode.WebviewView;
    private client: FreecodeClient;
    private sessionId: string;
    private activeCall: any = null;
    private pendingHitlResolver: ((result: { choice: 'Accept' | 'Discard'; edits?: Record<string, string> }) => void) | null = null;

    constructor(
        private readonly _extensionUri: vscode.Uri,
    ) {
        this.client = new FreecodeClient('127.0.0.1:50051');
        this.sessionId = 'sess_' + Math.random().toString(36).substring(2, 11);
    }

    private getMemoryPaths() {
        const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
        const projectPath = workspaceFolder ? path.join(workspaceFolder, '.freecode', 'project_memory.json') : null;
        
        const homeDir = os.homedir();
        const globalPath = path.join(homeDir, '.freecode', 'global_memory.json');
        
        return { projectPath, globalPath };
    }

    private readMemoryFile(filePath: string): any[] {
        try {
            if (fs.existsSync(filePath)) {
                const content = fs.readFileSync(filePath, 'utf8');
                return JSON.parse(content);
            }
        } catch (err) {
            console.error(`Error reading memory file ${filePath}:`, err);
        }
        return [];
    }

    private writeMemoryFile(filePath: string, data: any[]) {
        try {
            const dir = path.dirname(filePath);
            if (!fs.existsSync(dir)) {
                fs.mkdirSync(dir, { recursive: true });
            }
            fs.writeFileSync(filePath, JSON.stringify(data, null, 2), 'utf8');
        } catch (err) {
            console.error(`Error writing memory file ${filePath}:`, err);
        }
    }

    public resolveWebviewView(
        webviewView: vscode.WebviewView,
        context: vscode.WebviewViewResolveContext,
        _token: vscode.CancellationToken,
    ) {
        this._view = webviewView;

        webviewView.webview.options = {
            enableScripts: true,
            localResourceRoots: [this._extensionUri]
        };

        webviewView.webview.html = this._getHtmlForWebview(webviewView.webview);

        // Listen for messages from Webview
        webviewView.webview.onDidReceiveMessage(async (data) => {
            switch (data.type) {
                case 'getScope': {
                    webviewView.webview.postMessage({ type: 'scope', scope: this.describeWriteScope() });
                    break;
                }
                case 'readConfig': {
                    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || '.';
                    const configPath = path.join(workspaceFolder, '.freecode', 'config.json');
                    let config = {};
                    if (fs.existsSync(configPath)) {
                        try {
                            config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
                        } catch (err) {
                            console.error('Error parsing config file:', err);
                        }
                    }
                    webviewView.webview.postMessage({
                        type: 'config',
                        config
                    });
                    break;
                }
                case 'writeConfig': {
                    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || '.';
                    const configDir = path.join(workspaceFolder, '.freecode');
                    if (!fs.existsSync(configDir)) {
                        fs.mkdirSync(configDir, { recursive: true });
                    }
                    const configPath = path.join(configDir, 'config.json');
                    try {
                        fs.writeFileSync(configPath, JSON.stringify(data.config, null, 2), 'utf8');
                    } catch (err) {
                        console.error('Error writing config file:', err);
                    }
                    break;
                }
                case 'exportTrajectory': {
                    const { trajectory, sessionId, isAuto } = data;
                    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || '.';
                    const trajectoriesDir = path.join(workspaceFolder, '.freecode', 'trajectories');
                    if (!fs.existsSync(trajectoriesDir)) {
                        fs.mkdirSync(trajectoriesDir, { recursive: true });
                    }
                    const sessId = sessionId || this.sessionId || 'default';
                    const filename = `trajectory_${sessId}.json`;
                    const filePath = path.join(trajectoriesDir, filename);
                    try {
                        fs.writeFileSync(filePath, JSON.stringify(trajectory, null, 2), 'utf8');
                        if (!isAuto) {
                            webviewView.webview.postMessage({
                                type: 'step',
                                status: 'success',
                                message: `Trajectory successfully exported to .freecode/trajectories/${filename}`
                            });
                        }
                    } catch (err: any) {
                        if (!isAuto) {
                            webviewView.webview.postMessage({
                                type: 'step',
                                status: 'error',
                                message: `Failed to export trajectory: ${err.message}`
                            });
                        }
                    }
                    break;
                }
                case 'getMemories': {
                    const { projectPath, globalPath } = this.getMemoryPaths();
                    const project = projectPath ? this.readMemoryFile(projectPath) : [];
                    const global = this.readMemoryFile(globalPath);
                    webviewView.webview.postMessage({
                        type: 'memories',
                        project,
                        global
                    });
                    break;
                }
                case 'saveMemory': {
                    const { memoryType, note } = data;
                    const { projectPath, globalPath } = this.getMemoryPaths();
                    const filePath = memoryType === 'project' ? projectPath : globalPath;
                    if (filePath) {
                        const memories = this.readMemoryFile(filePath);
                        const existingIdx = memories.findIndex(m => m.id === note.id);
                        if (existingIdx !== -1) {
                            memories[existingIdx].content = note.content;
                        } else {
                            memories.push(note);
                        }
                        this.writeMemoryFile(filePath, memories);
                    }
                    const project = projectPath ? this.readMemoryFile(projectPath) : [];
                    const global = this.readMemoryFile(globalPath);
                    webviewView.webview.postMessage({
                        type: 'memories',
                        project,
                        global
                    });
                    break;
                }
                case 'deleteMemory': {
                    const { memoryType, id } = data;
                    const { projectPath, globalPath } = this.getMemoryPaths();
                    const filePath = memoryType === 'project' ? projectPath : globalPath;
                    if (filePath) {
                        let memories = this.readMemoryFile(filePath);
                        memories = memories.filter(m => m.id !== id);
                        this.writeMemoryFile(filePath, memories);
                    }
                    const project = projectPath ? this.readMemoryFile(projectPath) : [];
                    const global = this.readMemoryFile(globalPath);
                    webviewView.webview.postMessage({
                        type: 'memories',
                        project,
                        global
                    });
                    break;
                }
                case 'ping': {
                    try {
                        const res = await this.client.ping();
                        webviewView.webview.postMessage({
                            type: 'step',
                            status: 'success',
                            message: `Connected to daemon. Version: ${res.version}`
                        });
                        webviewView.webview.postMessage({
                            type: 'connectionStatus',
                            connected: true
                        });
                    } catch (err: any) {
                        webviewView.webview.postMessage({
                            type: 'step',
                            status: 'error',
                            message: DAEMON_OFFLINE_MSG
                        });
                        webviewView.webview.postMessage({
                            type: 'connectionStatus',
                            connected: false
                        });
                    }
                    break;
                }
                case 'dispatch': {
                    const { prompt, mode, llmEndpoint, llmModel } = data;
                    webviewView.webview.postMessage({
                        type: 'step',
                        status: 'info',
                        message: `Analyzing intent in ${mode} mode...`
                    });
                    
                    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || '.';
                    this.writeAuditLog(workspaceFolder, 'dispatch_intent', {
                        prompt,
                        mode,
                        session_id: this.sessionId,
                        user: os.userInfo().username || 'operator'
                    });

                    let finalMessage = '';
                    let finalSessionId = this.sessionId;
                    let hasError = false;
                    let proposalCount = 0;
                    const proposalOld: Record<string, string> = {};
                    // RFC-002 Slice 2: `run` commands the daemon staged for approval this dispatch.
                    // On Accept we re-dispatch each (with approved_command) so the DAEMON executes it
                    // under its policy/container — never the extension.
                    const pendingRunCommands: string[] = [];

                    // RFC-006 T1: forward the active editor's selection (the `before` span) and its
                    // workspace-relative file. The daemon uses them only when the T1 fast-path is
                    // enabled and the turn classifies as a TrivialEdit; otherwise they're ignored.
                    const t1Editor = vscode.window.activeTextEditor;
                    const t1Selection = (t1Editor && !t1Editor.selection.isEmpty)
                        ? t1Editor.document.getText(t1Editor.selection) : '';
                    const t1File = t1Editor ? vscode.workspace.asRelativePath(t1Editor.document.uri, false) : '';

                    this.activeCall = this.client.dispatchIntent(
                        prompt,
                        mode,
                        workspaceFolder,
                        this.sessionId,
                        llmEndpoint,
                        llmModel,
                        (res) => {
                            if (res.status === 'token') {
                                webviewView.webview.postMessage({
                                    type: 'token',
                                    message: res.message
                                });
                            } else if (res.status === 'step') {
                                webviewView.webview.postMessage({
                                    type: 'step',
                                    status: 'info',
                                    message: res.message
                                });
                            } else if (res.status === 'status') {
                                finalMessage = res.message;
                                if (res.session_id) {
                                    finalSessionId = res.session_id;
                                    this.sessionId = res.session_id;
                                }
                            } else if (res.status === 'metrics') {
                                try {
                                    const parsed = JSON.parse(res.message);
                                    this.writeAuditLog(workspaceFolder, 'execution_metrics', parsed);
                                    webviewView.webview.postMessage({
                                        type: 'metrics',
                                        metrics: parsed
                                    });
                                } catch (err) {
                                    console.error('Error parsing metrics JSON:', err);
                                }
                            } else {
                                if (res.status === 'proposal') {
                                    proposalCount++;
                                    // Remember the file's content at proposal time so we
                                    // can detect a conflicting edit before we overwrite it.
                                    try {
                                        const p = JSON.parse(res.message);
                                        if (p && p.kind === 'run-command' && typeof p.command === 'string') {
                                            pendingRunCommands.push(p.command);
                                        } else if (p && typeof p.filePath === 'string') {
                                            proposalOld[p.filePath] = typeof p.oldContent === 'string' ? p.oldContent : '';
                                        }
                                    } catch { /* ignore malformed proposal */ }
                                }
                                webviewView.webview.postMessage({
                                    type: res.status,
                                    message: res.message
                                });
                            }
                        },
                        () => {
                            this.activeCall = null;
                            if (hasError) return;

                            if (mode === 'auto') {
                                vscode.window.showInformationMessage(`FreeCode: Autonomous task completed for session ${finalSessionId}.`);
                            }

                            this.writeAuditLog(workspaceFolder, 'dispatch_finished', {
                                session_id: finalSessionId,
                                success: true
                            });

                            webviewView.webview.postMessage({
                                type: 'step',
                                status: 'success',
                                message: `Daemon finished processing.`
                            });
                            
                            webviewView.webview.postMessage({
                                type: 'response',
                                status: 'success',
                                message: finalMessage,
                                sessionId: finalSessionId
                            });

                            // HITL confirmation: the daemon STAGED the proposals (it
                            // wrote nothing to disk). The webview already renders the
                            // diff; materialize on Accept, Discard is a disk no-op.
                            if (mode === 'hitl' && proposalCount > 0) {
                                (async () => {
                                    // Tell webview that HITL decision is pending
                                    webviewView.webview.postMessage({
                                        type: 'hitlPending',
                                        sessionId: finalSessionId
                                    });

                                     const webviewPromise = new Promise<{ choice: 'Accept' | 'Discard'; edits?: Record<string, string> }>((resolve) => {
                                         this.pendingHitlResolver = resolve;
                                     });
                                     const vscodePromise = vscode.window.showInformationMessage(
                                         `FreeCode staged ${proposalCount} proposal(s). Review the diff and accept or discard.`,
                                         "Accept",
                                         "Discard"
                                     ).then(val => ({ choice: (val === 'Accept' ? 'Accept' : 'Discard') as 'Accept' | 'Discard' }));

                                     const result = await Promise.race<{ choice: 'Accept' | 'Discard'; edits?: Record<string, string> }>([webviewPromise, vscodePromise]);
                                     this.pendingHitlResolver = null;
                                     const choice = result.choice;
                                     
                                     this.writeAuditLog(workspaceFolder, 'hitl_decision', {
                                         decision: choice === 'Discard' ? 'discard' : 'accept',
                                         files: result.edits ? Object.keys(result.edits) : [],
                                         user: os.userInfo().username || 'operator'
                                     });

                                     if (choice === 'Discard') {
                                         // Nothing was written — discarding is a disk no-op.
                                         webviewView.webview.postMessage({
                                             type: 'step',
                                             status: 'info',
                                             message: `Proposals discarded. Nothing was written.`
                                         });
                                     } else {
                                         // Materialize the (possibly edited) staged content,
                                         // but first detect files that changed on disk since
                                         // the proposal was made (concurrent edits) — never
                                         // clobber them silently.
                                         const edits = result.edits || {};
                                         const conflicts: string[] = [];
                                         for (const relPath of Object.keys(edits)) {
                                             const filePath = path.join(workspaceFolder, relPath);
                                             const current = fs.existsSync(filePath) ? fs.readFileSync(filePath, 'utf8') : '';
                                             const proposedBase = proposalOld[relPath] ?? '';
                                             if (current !== proposedBase) { conflicts.push(relPath); }
                                         }

                                         let overwriteConflicts = true;
                                         if (conflicts.length > 0) {
                                             const pick = await vscode.window.showWarningMessage(
                                                 `${conflicts.length} file(s) changed on disk since FreeCode proposed them: ${conflicts.join(', ')}. Overwrite with the proposed version?`,
                                                 { modal: true },
                                                 'Overwrite',
                                                 'Skip conflicted'
                                             );
                                             overwriteConflicts = pick === 'Overwrite';
                                             this.writeAuditLog(workspaceFolder, 'hitl_conflict', {
                                                 files: conflicts,
                                                 resolution: overwriteConflicts ? 'overwrite' : 'skip',
                                                 user: os.userInfo().username || 'operator'
                                             });
                                         }

                                         let wrote = 0;
                                         let skipped = 0;
                                         for (const [relPath, newContent] of Object.entries(edits)) {
                                             if (!overwriteConflicts && conflicts.includes(relPath)) { skipped++; continue; }
                                             try {
                                                 const filePath = path.join(workspaceFolder, relPath);
                                                 const dir = path.dirname(filePath);
                                                 if (!fs.existsSync(dir)) { fs.mkdirSync(dir, { recursive: true }); }
                                                 fs.writeFileSync(filePath, newContent, 'utf8');
                                                 wrote++;
                                             } catch (e: any) {
                                                 console.error(`Failed to write accepted file ${relPath}:`, e);
                                             }
                                         }
                                         webviewView.webview.postMessage({
                                             type: 'step',
                                             status: skipped > 0 ? 'info' : 'success',
                                             message: skipped > 0
                                                 ? `Accepted — wrote ${wrote} file(s); skipped ${skipped} conflicted (changed on disk).`
                                                 : `Accepted — wrote ${wrote} file(s).`
                                         });

                                         // Verify only now (post-approval) — and only if files actually
                                         // changed (a command-only approval writes nothing to compile).
                                         const checkResult = wrote > 0
                                             ? await this.runCompileCheck(workspaceFolder)
                                             : { passed: true, details: 'no files changed' };
                                         const verdictData = {
                                             gateName: "Post-Approval Compiler Gate",
                                             rule: "Accepted changes must compile",
                                             passed: checkResult.passed,
                                             details: checkResult.details
                                         };
                                         webviewView.webview.postMessage({
                                             type: 'gate_verdict',
                                             message: JSON.stringify(verdictData)
                                         });

                                         if (!checkResult.passed) {
                                             webviewView.webview.postMessage({
                                                 type: 'step',
                                                 status: 'error',
                                                 message: `Compiler check failed after user edits: ${checkResult.details}`
                                             });
                                         }
                                     }

                                    // RFC-002 Slice 2: on Accept, execute any approved `run` commands by
                                    // RE-DISPATCHING each to the daemon (approved_command) so its policy,
                                    // container and timeout apply. The extension never runs shell itself.
                                    if (choice !== 'Discard' && pendingRunCommands.length > 0) {
                                        for (const command of pendingRunCommands) {
                                            this.writeAuditLog(workspaceFolder, 'run_approved', {
                                                command,
                                                user: os.userInfo().username || 'operator'
                                            });
                                            webviewView.webview.postMessage({
                                                type: 'step', status: 'info', message: `Executing approved command: ${command}`
                                            });
                                            await new Promise<void>((resolve) => {
                                                this.client.dispatchIntent(
                                                    '', mode, workspaceFolder, finalSessionId, llmEndpoint, llmModel,
                                                    (res) => { webviewView.webview.postMessage({ type: res.status, message: res.message }); },
                                                    () => resolve(),
                                                    (err) => {
                                                        webviewView.webview.postMessage({ type: 'step', status: 'error', message: `Run failed: ${err?.message || err}` });
                                                        resolve();
                                                    },
                                                    command
                                                );
                                            });
                                        }
                                    }

                                    // Tell webview the decision has been applied
                                    webviewView.webview.postMessage({
                                        type: 'hitlDecisionApplied',
                                        decision: choice === 'Discard' ? 'discard' : 'accept',
                                        sessionId: finalSessionId
                                    });
                                })();
                            } else if (mode !== 'hitl') {
                                // Auto/chat: the daemon may have left a backup dir; clean it on success.
                                const backupDir = path.join(workspaceFolder, '.freecode', 'backups', finalSessionId);
                                if (fs.existsSync(backupDir)) {
                                    fs.rmSync(backupDir, { recursive: true, force: true });
                                }
                            }
                        },
                        (err) => {
                            this.activeCall = null;
                            hasError = true;
                            const isCancelled = err.code === 1 || err.message?.includes('CANCELLED');

                            if (mode === 'auto') {
                                if (isCancelled) {
                                    vscode.window.showInformationMessage(`FreeCode: Autonomous task for session ${this.sessionId} was cancelled.`);
                                } else {
                                    vscode.window.showErrorMessage(`FreeCode: Autonomous task failed for session ${this.sessionId}.`);
                                }
                            }

                            this.writeAuditLog(workspaceFolder, 'dispatch_finished', {
                                session_id: this.sessionId,
                                success: false,
                                error: isCancelled ? 'Cancelled by user' : (err.message || err)
                            });
                            if (isCancelled) {
                                webviewView.webview.postMessage({
                                    type: 'response',
                                    status: 'cancelled',
                                    message: `Execution stopped.`,
                                    sessionId: this.sessionId
                                });
                            } else {
                                // One classified error (no duplicate step+response, no raw grpc-js leak,
                                // no double "Error:" prefix). errorKind drives the recovery affordance.
                                const e = classifyDaemonError(err);
                                webviewView.webview.postMessage({
                                    type: 'response',
                                    status: 'error',
                                    errorKind: e.kind,
                                    message: e.message,
                                    sessionId: this.sessionId
                                });
                            }

                            if (isCancelled) {
                                const backupDir = path.join(workspaceFolder, '.freecode', 'backups', finalSessionId);
                                if (fs.existsSync(backupDir)) {
                                    this.restoreBackup(backupDir, workspaceFolder);
                                    webviewView.webview.postMessage({
                                        type: 'step',
                                        status: 'info',
                                        message: `Changes discarded. Original files restored.`
                                    });
                                }
                            }
                        },
                        '',          // approvedCommand: this is a normal intent, not a run re-dispatch
                        t1Selection, // RFC-006 T1: the IDE-selected before-span
                        t1File       // RFC-006 T1: its workspace-relative file
                    );
                    break;
                }
                case 'stop': {
                    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || '.';
                    this.writeAuditLog(workspaceFolder, 'stop_intent', {
                        session_id: this.sessionId,
                        user: os.userInfo().username || 'operator'
                    });
                    if (this.activeCall) {
                        try {
                            this.activeCall.cancel();
                        } catch (err) {
                            console.error('Failed to cancel active call:', err);
                        }
                        this.activeCall = null;
                    }
                    break;
                }
                case 'applyAst': {
                    const { filePath, symbolName, newContent } = data;
                    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || '.';
                    webviewView.webview.postMessage({
                        type: 'step',
                        status: 'info',
                        message: `AST edit for symbol '${symbolName}' in ${filePath}`
                    });
                    try {
                        let absolutePath = filePath;
                        if (!path.isAbsolute(filePath)) {
                            absolutePath = path.join(workspaceFolder, filePath);
                        }
                        const res = await this.client.applyAstEdit(absolutePath, symbolName, newContent);
                        this.writeAuditLog(workspaceFolder, 'apply_ast_edit', {
                            filePath,
                            symbolName,
                            success: res.success,
                            message: res.message
                        });
                        if (res.success) {
                            webviewView.webview.postMessage({
                                type: 'step',
                                status: 'success',
                                message: `AST Edit Applied: ${res.message}`
                            });
                        } else {
                            webviewView.webview.postMessage({
                                type: 'step',
                                status: 'error',
                                message: `AST Edit Refused: ${res.message}`
                            });
                        }
                    } catch (err: any) {
                        this.writeAuditLog(workspaceFolder, 'apply_ast_edit', {
                            filePath,
                            symbolName,
                            success: false,
                            error: err.message || err
                        });
                        webviewView.webview.postMessage({
                            type: 'step',
                            status: 'error',
                            message: `AST Error: ${err.message || err}`
                        });
                    }
                    break;
                }
                case 'modeChanged': {
                    const { oldMode, newMode } = data;
                    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || '.';
                    this.writeAuditLog(workspaceFolder, 'mode_switch', {
                        from: oldMode,
                        to: newMode,
                        user: os.userInfo().username || 'operator'
                    });
                    break;
                }
                case 'clearChat': {
                    this.sessionId = 'sess_' + Math.random().toString(36).substring(2, 11);
                    break;
                }
                case 'hitlResponse': {
                    const { decision, edits } = data;
                    if (this.pendingHitlResolver) {
                        this.pendingHitlResolver({
                            choice: decision === 'accept' ? 'Accept' : 'Discard',
                            edits
                        });
                    }
                    break;
                }
                case 'openFile': {
                    const { filePath } = data;
                    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri;
                    if (workspaceFolder) {
                        const fileUri = vscode.Uri.joinPath(workspaceFolder, filePath);
                        // `filePath` comes from a model-authored markdown link. `Uri.joinPath`
                        // happily resolves `..`, so `[log](../../.ssh/id_rsa)` would open a file
                        // outside the workspace on click. Confine it to the blast radius, the
                        // same rule the daemon enforces on writes (resolve_in_workspace).
                        const root = path.resolve(workspaceFolder.fsPath);
                        const target = path.resolve(fileUri.fsPath);
                        if (target !== root && !target.startsWith(root + path.sep)) {
                            vscode.window.showWarningMessage(
                                `FreeCode: refused to open "${filePath}" — it resolves outside the workspace.`
                            );
                            break;
                        }
                        if (!fs.existsSync(fileUri.fsPath)) {
                            // The model often references files in prose that don't exist on
                            // disk; degrade gracefully instead of a raw ENOENT error.
                            vscode.window.showWarningMessage(`FreeCode: "${filePath}" was mentioned but isn't in the workspace.`);
                            break;
                        }
                        vscode.workspace.openTextDocument(fileUri).then(
                            doc => {
                                vscode.window.showTextDocument(doc);
                            },
                            () => {
                                vscode.window.showWarningMessage(`FreeCode: couldn't open ${filePath}.`);
                            }
                        );
                    }
                    break;
                }
                case 'checkConnection': {
                    try {
                        await this.client.ping();
                        webviewView.webview.postMessage({
                            type: 'connectionStatus',
                            connected: true
                        });
                    } catch (e) {
                        webviewView.webview.postMessage({
                            type: 'connectionStatus',
                            connected: false
                        });
                    }
                    break;
                }
                case 'getGitStatus': {
                    try {
                        const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || '.';
                        const status = await this.client.getGitStatus(workspaceFolder);
                        webviewView.webview.postMessage({
                            type: 'gitStatus',
                            status
                        });
                    } catch (err) {
                        // ignore error
                    }
                    break;
                }
                case 'openDiff': {
                    const { filePath } = data;
                    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri;
                    if (workspaceFolder) {
                        const fileUri = vscode.Uri.joinPath(workspaceFolder, filePath);
                        vscode.commands.executeCommand('git.openChange', fileUri).then(
                            undefined,
                            err => {
                                // fallback to standard editor
                                vscode.workspace.openTextDocument(fileUri).then(
                                    doc => { vscode.window.showTextDocument(doc); },
                                    () => { vscode.window.showWarningMessage(`FreeCode: couldn't open diff for ${filePath}.`); }
                                );
                            }
                        );
                    }
                    break;
                }
                case 'getActiveFilePath': {
                    const activeEditor = vscode.window.activeTextEditor;
                    let relPath = '';
                    if (activeEditor) {
                        const fsPath = activeEditor.document.uri.fsPath;
                        const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
                        if (workspaceFolder && fsPath.startsWith(workspaceFolder)) {
                            relPath = path.relative(workspaceFolder, fsPath);
                        } else {
                            relPath = path.basename(fsPath);
                        }
                    }
                    webviewView.webview.postMessage({
                        type: 'activeFilePath',
                        filePath: relPath
                    });
                    break;
                }
            }
        });
    }

    private restoreBackup(backupDir: string, workspaceDir: string) {
        if (!fs.existsSync(backupDir)) return;
        const restoreRecursive = (dir: string, relativePath: string = '') => {
            const files = fs.readdirSync(dir);
            for (const file of files) {
                const fullPath = path.join(dir, file);
                const relPath = path.join(relativePath, file);
                const stat = fs.statSync(fullPath);
                if (stat.isDirectory()) {
                    restoreRecursive(fullPath, relPath);
                } else {
                    if (file.endsWith('.created')) {
                        const originalRelPath = relPath.substring(0, relPath.length - 8);
                        const originalPath = path.join(workspaceDir, originalRelPath);
                        if (fs.existsSync(originalPath)) {
                            fs.unlinkSync(originalPath);
                        }
                    } else {
                        const originalPath = path.join(workspaceDir, relPath);
                        const originalDir = path.dirname(originalPath);
                        if (!fs.existsSync(originalDir)) {
                            fs.mkdirSync(originalDir, { recursive: true });
                        }
                        fs.copyFileSync(fullPath, originalPath);
                    }
                }
            }
        };
        restoreRecursive(backupDir);
        fs.rmSync(backupDir, { recursive: true, force: true });
    }


    private writeAuditLog(workspaceFolder: string, action: string, details: any) {
        try {
            const auditDir = path.join(workspaceFolder, '.freecode');
            if (!fs.existsSync(auditDir)) {
                fs.mkdirSync(auditDir, { recursive: true });
            }
            const auditPath = path.join(auditDir, 'audit_log.json');
            let logs: any[] = [];
            if (fs.existsSync(auditPath)) {
                try {
                    const content = fs.readFileSync(auditPath, 'utf8');
                    logs = JSON.parse(content);
                } catch (e) {
                    logs = [];
                }
            }
            const timestamp = new Date().toISOString();
            logs.push({
                timestamp,
                action,
                details
            });
            fs.writeFileSync(auditPath, JSON.stringify(logs, null, 2), 'utf8');
        } catch (e) {
            console.error('Failed to write audit log:', e);
        }
    }

    private async runCompileCheck(workspaceFolder: string): Promise<{ passed: boolean; details: string }> {
        return new Promise((resolve) => {
            let command = '';
            if (fs.existsSync(path.join(workspaceFolder, 'Cargo.toml'))) {
                command = 'cargo check';
            } else if (fs.existsSync(path.join(workspaceFolder, 'package.json'))) {
                let hasBuild = false;
                try {
                    const pkg = JSON.parse(fs.readFileSync(path.join(workspaceFolder, 'package.json'), 'utf8'));
                    hasBuild = !!(pkg.scripts && pkg.scripts.build);
                } catch { /* ignore malformed package.json */ }
                if (hasBuild) {
                    command = 'npm run build';
                } else if (fs.existsSync(path.join(workspaceFolder, 'tsconfig.json'))) {
                    command = 'npx tsc --noEmit';
                } else {
                    resolve({ passed: true, details: 'No build script or tsconfig.json found. Skipping compile check.' });
                    return;
                }
            } else {
                resolve({ passed: true, details: 'No Cargo.toml or package.json found. Skipping compile check.' });
                return;
            }

            cp.exec(command, { cwd: workspaceFolder }, (error, stdout, stderr) => {
                if (error) {
                    const details = (stderr || stdout || error.message).trim();
                    resolve({ passed: false, details });
                } else {
                    const details = (stdout || stderr || 'Compile check passed.').trim();
                    resolve({ passed: true, details });
                }
            });
        });
    }

    /**
     * Every location the daemon can write, so the panel can state it instead of the operator
     * having to trust it. Three of these are OUTSIDE the open repository — that is the whole
     * point of surfacing this: `~/.freecode/global_memory.json` holds model-authored text and
     * nobody currently sees that it exists.
     *
     * These paths are duplicated knowledge: the daemon is the source of truth and this list
     * mirrors it. Each entry names the daemon file that owns it, so a change there has an
     * obvious place to land. Keep them in sync — a permission display that lies is worse than
     * none at all.
     */
    private describeWriteScope() {
        const ws = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? null;
        const home = os.homedir();
        const entries = [
            ws && {
                path: ws,
                label: 'workspace',
                access: 'rw',
                inside: true,
                note: 'model edits — confined here, symlinks resolved (core.rs resolve_in_workspace)',
            },
            ws && {
                path: path.join(ws, '.freecode'),
                label: 'session state',
                access: 'rw',
                inside: true,
                note: 'pre-edit backups, per-project gate config, trajectories',
            },
            {
                path: path.join(home, '.freecode', 'global_memory.json'),
                label: 'cross-project memory',
                access: 'rw',
                inside: false,
                note: 'WRITTEN BY THE MODEL, outside this repo (core.rs)',
            },
            {
                path: path.join(home, '.freecode', 'config.json'),
                label: 'analyzer registry',
                access: 'ro',
                inside: false,
                note: 'read only, and never read from the repo — an untrusted checkout cannot register commands (analyzers.rs)',
            },
            {
                // Default only: the daemon honours $FREECODE_ROUTE_LOG, which this process
                // cannot observe (escalation.rs).
                path: path.join(home, 'Library', 'Logs', 'freecode-route.jsonl'),
                label: 'route telemetry',
                access: 'append',
                inside: false,
                note: 'one line per turn; overridable with $FREECODE_ROUTE_LOG',
            },
        ].filter(Boolean);
        return { root: ws, home, entries };
    }

    private _getHtmlForWebview(webview: vscode.Webview) {
        // A fresh nonce per render. The CSP admits exactly the one <script> carrying it, so
        // injected markup cannot introduce executable code even if it reached the DOM.
        // randomBytes, not Math.random: this is a security token, not a cache-buster.
        const nonce = crypto.randomBytes(16).toString('base64');
        return getWebviewHtml(getWebviewCss(), getWebviewJs(), webview.cspSource, nonce);
    }
}
