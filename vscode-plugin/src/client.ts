import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';
import * as path from 'path';
import * as fs from 'fs';

export class FreecodeClient {
    private client: any = null;
    private address: string;

    constructor(address: string = 'localhost:50051') {
        this.address = address;
        this.initClient();
    }

    private initClient() {
        // Resolve proto path dynamically (checking dist/ first, then dev fallbacks)
        let protoPath = path.join(__dirname, 'freecode.proto');
        if (!fs.existsSync(protoPath)) {
            protoPath = path.join(__dirname, '..', '..', 'proto', 'freecode.proto');
        }
        if (!fs.existsSync(protoPath)) {
            protoPath = path.join(__dirname, '..', 'proto', 'freecode.proto');
        }
        if (!fs.existsSync(protoPath)) {
            protoPath = path.join(__dirname, 'proto', 'freecode.proto');
        }

        try {
            const packageDefinition = protoLoader.loadSync(protoPath, {
                keepCase: true,
                longs: String,
                enums: String,
                defaults: true,
                oneofs: true,
            });
            const protoDescriptor = grpc.loadPackageDefinition(packageDefinition) as any;
            const freecode = protoDescriptor.freecode;
            if (freecode && freecode.FreecodeService) {
                this.client = new freecode.FreecodeService(
                    this.address,
                    grpc.credentials.createInsecure()
                );
            } else {
                console.error('FreecodeService not found in protobuf description.');
            }
        } catch (error) {
            console.error('Failed to load protobuf or initialize gRPC client:', error);
        }
    }

    public isReady(): boolean {
        return this.client !== null;
    }

    public ping(): Promise<{ version: string; status: string }> {
        return new Promise((resolve, reject) => {
            if (!this.client) {
                return reject(new Error('gRPC client is not initialized. Check proto file path and connection.'));
            }
            this.client.Ping({}, (err: any, response: any) => {
                if (err) {
                    reject(err);
                } else {
                    resolve(response);
                }
            });
        });
    }

    public dispatchIntent(
        prompt: string,
        mode: string,
        workspacePath: string = '.',
        sessionId: string = '',
        llmEndpoint: string = '',
        llmModel: string = '',
        onData: (data: { status: string; message: string; session_id: string }) => void,
        onEnd: () => void,
        onError: (err: any) => void,
        // RFC-002 Slice 2: set to a human-approved `run` command to execute it (the daemon skips the
        // LLM and re-validates the command). Empty for normal intents.
        approvedCommand: string = '',
        // RFC-006 T1: the IDE selection (the `before` span) + its workspace-relative file. When the
        // T1 fast-path is enabled and the turn is a TrivialEdit, the daemon uses these; else ignored.
        selection: string = '',
        file: string = ''
    ): any {
        if (!this.client) {
            onError(new Error('gRPC client is not initialized.'));
            return null;
        }
        const call = this.client.DispatchIntent({
            prompt,
            workspace_path: workspacePath,
            mode,
            session_id: sessionId,
            llm_endpoint: llmEndpoint,
            llm_model: llmModel,
            approved_command: approvedCommand,
            selection,
            file
        });
        call.on('data', (response: any) => {
            onData(response);
        });
        call.on('end', () => {
            onEnd();
        });
        call.on('error', (err: any) => {
            onError(err);
        });
        return call;
    }

    public applyAstEdit(filePath: string, symbolName: string, newContent: string): Promise<{ success: boolean; message: string }> {
        return new Promise((resolve, reject) => {
            if (!this.client) {
                return reject(new Error('gRPC client is not initialized.'));
            }
            this.client.ApplyAstEdit({ file_path: filePath, symbol_name: symbolName, new_content: newContent }, (err: any, response: any) => {
                if (err) {
                    reject(err);
                } else {
                    resolve(response);
                }
            });
        });
    }

    public getGitStatus(workspacePath: string): Promise<{ is_inside_repo: boolean; branch: string; modified_files: string[]; added_files: string[]; deleted_files: string[] }> {
        return new Promise((resolve, reject) => {
            if (!this.client) {
                return reject(new Error('gRPC client is not initialized.'));
            }
            this.client.GetGitStatus({ workspace_path: workspacePath }, (err: any, response: any) => {
                if (err) {
                    reject(err);
                } else {
                    resolve(response);
                }
            });
        });
    }
}
