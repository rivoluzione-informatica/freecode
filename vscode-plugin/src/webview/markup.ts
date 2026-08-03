export function getWebviewHtml(css: string, js: string, cspSource: string = '', nonce: string = ''): string {
    return `<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <!-- Defense-in-depth: even if some model-controlled string slips past escaping,
         block remote script/style/image loads and any network exfiltration.
         script-src is nonce-based, NOT 'unsafe-inline': every handler is delegated from
         data-* attributes (see fcDispatch), so injected markup has no way to run code even
         if it were to survive escapeHtml. -->
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${cspSource} 'unsafe-inline'; script-src 'nonce-${nonce}'; img-src ${cspSource} data:; font-src ${cspSource}; connect-src 'none';">
    <title>FreeCode Assistant</title>
    <style>
        ${css}
    </style>
</head>
<body>
    <div class="header">
        <div class="header-actions" style="width: 100%;">
            <!-- Git Status Toggle Icon Button -->
            <button id="gitToggleBtn" class="header-btn" data-action="toggleGitPanel" title="Toggle Git Status">
                <svg class="header-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="6" y1="3" x2="6" y2="15"></line>
                    <circle cx="18" cy="6" r="3"></circle>
                    <circle cx="6" cy="18" r="3"></circle>
                    <path d="M18 9a9 9 0 0 1-9 9"></path>
                </svg>
            </button>

            <!-- Memory Toggle Icon Button -->
            <button id="memoryToggleBtn" class="header-btn" data-action="toggleMemoryPanel" title="Toggle Memory Dashboard">
                <svg class="header-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"></path>
                </svg>
            </button>

            <!-- Harness Observability Toggle Icon Button -->
            <button id="harnessToggleBtn" class="header-btn" data-action="toggleHarnessPanel" title="Toggle Harness Observability">
                <svg class="header-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="18" y1="20" x2="18" y2="10"></line>
                    <line x1="12" y1="20" x2="12" y2="4"></line>
                    <line x1="6" y1="20" x2="6" y2="14"></line>
                </svg>
            </button>

            <!-- PIC-10: Pipeline strip view toggle (compact / vertical / hidden) -->
            <button id="fcStripBtn" class="header-btn" data-action="fcCycleMode" title="Pipeline view (click to cycle: compact / full / hidden)">
                <svg class="header-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="3" y1="12" x2="21" y2="12"></line>
                    <circle cx="6" cy="12" r="1.7"></circle>
                    <circle cx="12" cy="12" r="1.7"></circle>
                    <circle cx="18" cy="12" r="1.7"></circle>
                </svg>
            </button>

            <!-- AST Edit Toggle Icon Button -->
            <button id="astBtn" class="header-btn" data-action="toggleAstSection" title="Toggle AST Edit">
                <svg class="header-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <polyline points="16 18 22 12 16 6"></polyline>
                    <polyline points="8 6 2 12 8 18"></polyline>
                </svg>
            </button>
            
             <!-- Settings Toggle Icon Button -->
             <button id="settingsToggleBtn" class="header-btn" data-action="toggleSettingsPanel" title="Settings">
                 <svg class="header-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                     <circle cx="12" cy="12" r="3"></circle>
                     <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"></path>
                 </svg>
             </button>

             <!-- Export Trajectory Icon Button -->
             <button id="exportTrajectoryBtn" class="header-btn" data-action="exportTrajectory" title="Export Session Trajectory">
                 <svg class="header-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                     <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path>
                     <polyline points="7 10 12 15 17 10"></polyline>
                     <line x1="12" y1="15" x2="12" y2="3"></line>
                 </svg>
             </button>

             <!-- Status Badge (just the dot, pushed to the far right) -->
             <div id="statusBadge" class="status-badge checking" data-action="pingDaemon" title="FreeCode daemon status — click to re-check" style="margin-left: auto;">
                 <span class="status-dot"></span>
                 <span id="statusText" class="status-text">Checking…</span>
             </div>
         </div>
     </div>

    <!-- Collapsible Memory Panel -->
    <div id="memoryPanel" class="git-panel" style="position: sticky; top: 36px; z-index: 84; display: none; padding: 12px 14px; background: var(--input-bg);">
        <div class="ast-card-header" style="margin-bottom: 8px;">
            <h4>Memory Dashboard</h4>
            <span style="cursor: pointer; font-weight: bold;" data-action="toggleMemoryPanel">&times;</span>
        </div>
        
        <div style="display: flex; flex-direction: column; gap: 8px;">
            <!-- Project Memory Section (Expanded by Default) -->
            <div class="memory-section">
                <div class="memory-section-header" data-action="toggleMemorySubSection" data-a1="project" style="display: flex; justify-content: space-between; align-items: center; cursor: pointer; user-select: none; font-weight: 600; color: var(--text-muted); font-size: 11px; margin-bottom: 4px;">
                    <span>Project Memory</span>
                    <span id="projectMemoryChevron">▼</span>
                </div>
                
                <div id="projectMemoryContent" style="display: flex; flex-direction: column; gap: 4px; padding-left: 4px;">
                    <div id="projectMemoryList" style="display: flex; flex-direction: column; gap: 4px; max-height: 120px; overflow-y: auto;">
                        <!-- Dynamically populated -->
                    </div>
                    <div style="display: flex; gap: 4px; margin-top: 4px;">
                        <input type="text" id="addProjectMemoryInput" class="input-text" style="margin-bottom: 0; padding: 3px 6px;" placeholder="Add project note...">
                        <button class="btn-secondary" style="width: auto; padding: 3px 8px; white-space: nowrap;" data-action="addMemoryNote" data-a1="project">+</button>
                    </div>
                </div>
            </div>
            
            <!-- Cross-Project Memory Section (Collapsed by Default) -->
            <div class="memory-section" style="border-top: 1px solid var(--card-border); padding-top: 8px;">
                <div class="memory-section-header" data-action="toggleMemorySubSection" data-a1="global" style="display: flex; justify-content: space-between; align-items: center; cursor: pointer; user-select: none; font-weight: 600; color: var(--text-muted); font-size: 11px; margin-bottom: 4px;">
                    <span>Cross-Project Memory</span>
                    <span id="globalMemoryChevron">▲</span>
                </div>
                
                <div id="globalMemoryContent" style="display: none; flex-direction: column; gap: 4px; padding-left: 4px;">
                    <div id="globalMemoryList" style="display: flex; flex-direction: column; gap: 4px; max-height: 120px; overflow-y: auto;">
                        <!-- Dynamically populated -->
                    </div>
                    <div style="display: flex; gap: 4px; margin-top: 4px;">
                        <input type="text" id="addGlobalMemoryInput" class="input-text" style="margin-bottom: 0; padding: 3px 6px;" placeholder="Add global note...">
                        <button class="btn-secondary" style="width: auto; padding: 3px 8px; white-space: nowrap;" data-action="addMemoryNote" data-a1="global">+</button>
                    </div>
                </div>
            </div>
        </div>
        <div class="panel-resizer" data-mousedown="initResize" data-a1="memoryPanel"></div>
    </div>

    <!-- Collapsible Settings Panel -->
    <div id="settingsPanel" class="git-panel" style="position: sticky; top: 36px; z-index: 85; display: none; padding: 12px 14px; background: var(--input-bg);">
        <div class="ast-card-header" style="margin-bottom: 8px;">
            <h4>Settings</h4>
            <span style="cursor: pointer; font-weight: bold;" data-action="toggleSettingsPanel">&times;</span>
        </div>
        <div style="display: flex; flex-direction: column; gap: 6px;">
            <div class="param-row" style="flex-direction: column; align-items: flex-start; gap: 4px;">
                <label style="font-size: 10px; color: var(--text-muted);">LLM Endpoint URL:</label>
                <input type="text" id="settingLlmEndpoint" class="input-text" style="width: 100%; box-sizing: border-box;" placeholder="http://127.0.0.1:1234/v1/chat/completions">
            </div>
            <div class="param-row" style="flex-direction: column; align-items: flex-start; gap: 4px;">
                <label style="font-size: 10px; color: var(--text-muted);">LLM Model Name:</label>
                <input type="text" id="settingLlmModel" class="input-text" style="width: 100%; box-sizing: border-box;" placeholder="gemma-4-e2b-it-mlx">
            </div>
            <div class="param-row" style="flex-direction: column; align-items: flex-start; gap: 4px;">
                <label style="font-size: 10px; color: var(--text-muted);">Excluded Files (glob patterns, comma-separated):</label>
                <input type="text" id="settingExcludedFiles" class="input-text" style="width: 100%; box-sizing: border-box;" placeholder="*.log, tmp/*, dist/*">
            </div>
            <div class="param-row" style="display: flex; align-items: center; gap: 6px; margin: 4px 0;">
                <input type="checkbox" id="settingMonotonicity" style="margin: 0; width: auto; cursor: pointer;">
                <label for="settingMonotonicity" style="font-size: 10px; color: var(--text-muted); cursor: pointer;" title="Only export successful trajectories (Phase 3 Trajectory Curation)">Monotonicity Curation (Only export successful runs)</label>
            </div>
            <button class="btn-secondary" style="margin-top: 4px;" data-action="saveSettings">
                <span>Save Settings</span>
            </button>
        </div>
    </div>

    <!-- Collapsible Git Status Panel -->
    <div id="gitPanel" class="git-panel" style="position: sticky; top: 36px; z-index: 85; display: none; padding: 12px 14px; background: var(--input-bg);">
        <div class="ast-card-header" style="margin-bottom: 8px;">
            <h4 id="gitPanelTitle">Git Status</h4>
            <span class="git-refresh-btn" title="Refresh Git Status" data-action="refreshGitStatus" style="margin-left: auto; margin-right: 8px; cursor: pointer;">↻</span>
            <span style="cursor: pointer; font-weight: bold;" data-action="toggleGitPanel">&times;</span>
        </div>
        <div id="gitFileList" class="git-file-list">
            <!-- Dynamically populated list -->
        </div>
        <div class="panel-resizer" data-mousedown="initResize" data-a1="gitFileList"></div>
    </div>

    <!-- Collapsible Harness Observability Panel -->
    <div id="harnessPanel" class="git-panel" style="position: sticky; top: 36px; z-index: 85; display: none;">
        <div class="harness-row">
            <div class="harness-column">
                <div class="harness-item" title="Expected or actual net payoff (s*V - e*L - Cost)">
                    <span class="harness-label">Payoff:</span>
                    <span id="valPayoff" class="harness-val">€0.00</span>
                </div>
                <div class="harness-item" title="Total cost">
                    <span class="harness-label">Cost:</span>
                    <span id="valCost" class="harness-val" style="color: var(--text-main);">€0.00</span>
                </div>
            </div>
            <div class="harness-column">
                <div class="harness-item" title="Model confidence">
                    <span class="harness-label">Conf.:</span>
                    <span id="valConfidence" class="harness-val" style="color: var(--text-main);">80.0%</span>
                </div>
                <div class="harness-item" title="Execution attempts">
                    <span class="harness-label">Att.:</span>
                    <span id="valAttempts" class="harness-val" style="color: var(--text-main);">1.00</span>
                </div>
            </div>
            <div class="harness-column-actions">
                <span id="harnessModeBadge" class="harness-badge" data-action="toggleParamsSection">Expected</span>
                <span class="harness-config-btn" data-action="toggleParamsSection">⚙ Settings</span>
            </div>
        </div>
        
        <!-- Session Cumulative Cost Section -->
        <div id="cumulativeCostCard" class="harness-row" style="border-top: 1px solid var(--card-border); padding-top: 6px; margin-top: 4px;">
            <div class="harness-column" style="flex: 1.5;">
                <div class="harness-item" title="Cumulative token cost for this session (prompt + completion)">
                    <span class="harness-label">Tokens:</span>
                    <span id="valSessionTokens" class="harness-val">0</span>
                </div>
                <div class="harness-item" title="Cumulative latency (total model latency)">
                    <span class="harness-label">Latency:</span>
                    <span id="valSessionLatency" class="harness-val">0.0s</span>
                </div>
            </div>
            <div class="harness-column" style="flex: 1.5;">
                <div class="harness-item" title="Total number of LLM API calls in this session">
                    <span class="harness-label">LLM Calls:</span>
                    <span id="valSessionCalls" class="harness-val">0</span>
                </div>
                <div class="harness-item" title="Total cumulative spent based on settings">
                    <span class="harness-label">Spent:</span>
                    <span id="valSessionSpent" class="harness-val" style="color: #4fc3f7;">€0.00</span>
                </div>
            </div>
            <div class="harness-column-actions" style="justify-content: center;">
                <span class="harness-badge" style="background: var(--button-bg-secondary); color: var(--text-main); font-size: 9px; padding: 2px 4px;">Cumulative</span>
            </div>
        </div>
        <div id="paramsContent" class="params-content">
            <div class="param-row">
                <label title="Monetary value of correctly resolved task">Task Value (V):</label>
                <input type="number" id="paramV" value="10.00" step="0.5" data-change="calculateHarnessMath">
            </div>
            <div class="param-row">
                <label title="Monetary loss if task is resolved with an undetected error">Error Loss (L):</label>
                <input type="number" id="paramL" value="20.00" step="1" data-change="calculateHarnessMath">
            </div>
            <div class="param-row">
                <label title="Cost of operator time or latency (€/second)">Time Value (κ):</label>
                <input type="number" id="paramKappa" value="0.10" step="0.01" data-change="calculateHarnessMath">
            </div>
            <div class="param-row">
                <label title="Cost of a single core model generation call">Gen Cost (C_gen):</label>
                <input type="number" id="paramCgen" value="0.20" step="0.05" data-change="calculateHarnessMath">
            </div>
            <div class="param-row">
                <label title="Cost of a single verifier/compiler call">Ver Cost (C_ver):</label>
                <input type="number" id="paramCver" value="0.02" step="0.01" data-change="calculateHarnessMath">
            </div>
            <div class="param-row">
                <label title="Model confidence g (calibrated probability of correctness)">Confidence (g):</label>
                <input type="range" id="paramG" min="0.1" max="1.0" step="0.01" value="0.80" data-input="rangeInput" data-a1="G">
                <span class="range-val" id="valG">80%</span>
            </div>
            <div class="param-row">
                <label title="False rejection probability of the gate on correct responses">False Rejects (α):</label>
                <input type="range" id="paramAlpha" min="0" max="0.5" step="0.01" value="0.05" data-input="rangeInput" data-a1="Alpha">
                <span class="range-val" id="valAlpha">0.05</span>
            </div>
            <div class="param-row">
                <label title="False acceptance probability of the gate on incorrect responses">False Positives (β):</label>
                <input type="range" id="paramBeta" min="0" max="0.5" step="0.01" value="0.02" data-input="rangeInput" data-a1="Beta">
                <span class="range-val" id="valBeta">0.02</span>
            </div>
            <div class="param-row">
                <label title="Maximum number of correction attempts (R)">Max Attempts (R):</label>
                <input type="number" id="paramR" value="3" min="1" max="10" step="1" data-change="calculateHarnessMath">
            </div>
        </div>
    </div>

    <!-- Collapsible AST Drawer -->
    <div id="astCard" class="ast-card">
        <div class="ast-card-header">
            <h4>AST Edit</h4>
            <span style="cursor: pointer; font-weight: bold;" data-action="toggleAstSection">&times;</span>
        </div>
        <div style="display: flex; flex-direction: column; gap: 4px;">
            <input type="text" id="filePathInput" class="input-text" placeholder="File Path (e.g. src/auth.rs)">
            <input type="text" id="symbolInput" class="input-text" placeholder="Symbol Name (e.g. login_user)">
            <textarea id="astContentInput" class="textarea" style="min-height: 60px;" placeholder="New function content..."></textarea>
            <button id="applyAstBtn" class="btn-secondary" data-action="applyAstEdit">
                <span>Apply AST Edit</span>
                <span id="astSpinner" class="spinner" style="display: none;"></span>
            </button>
        </div>
        <div class="panel-resizer" data-mousedown="initResize" data-a1="astContentInput"></div>
    </div>

    <!-- Chat Message Container -->
    <div id="chatContainer" class="chat-container">
        <div class="message assistant">
            <div class="message-sender">FreeCode</div>
            <div class="message-bubble" id="greetingBubble">
                FreeCode is ready 🏴‍☠️
            </div>
        </div>
    </div>

    <!-- Bottom Chat Input Panel -->
    <div class="bottom-panel">
        <!-- Write scope. Answers "what CAN happen" and stays still; the pipeline strip is the
             only animated element and answers "what IS happening". One moving thing at a time. -->
        <div id="scopeBar" class="scope-bar">
            <div class="scope-summary" data-action="toggleScope" title="Everywhere FreeCode can write">
                <span class="scope-icon">⌁</span>
                <span id="scopeSummaryText" class="scope-summary-text">checking scope…</span>
                <span id="scopeChevron" class="scope-chevron">▾</span>
            </div>
            <div id="scopeDetail" class="scope-detail"></div>
        </div>
        <div class="input-container">
            <textarea id="chatInput" class="chat-input" placeholder="Ask anything, e.g. refactor auth..." data-keydown="handleKeyPress"></textarea>
            <button id="sendBtn" class="send-btn" data-action="sendIntent">
                <svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="12" y1="19" x2="12" y2="5"></line>
                    <polyline points="5 12 12 5 19 12"></polyline>
                </svg>
            </button>
            <button id="stopBtn" class="stop-btn" data-action="stopIntent" style="display: none;">
                <svg class="icon" viewBox="0 0 24 24">
                    <path d="M18,18H6V6H18V18Z" stroke="currentColor" stroke-width="2" />
                </svg>
            </button>
        </div>
        <!-- The mode belongs to the message you are about to send, not to global settings —
             so it sits on the send row, not in a separate toolbar. -->
        <div class="compose-row">
            <div class="mode-picker">
                <button id="modeTrigger" class="mode-trigger" data-action="toggleModeMenu" title="How this message is executed">
                    <span class="mode-trigger-glyph">⌥</span>
                    <span id="modeTriggerLabel">Suggest</span>
                    <span class="mode-trigger-caret">▾</span>
                </button>
                <div id="modeMenu" class="mode-menu">
                    <button class="mode-option" id="mode-hitl" data-action="setMode" data-a1="hitl">
                        <span class="mode-option-dot"></span>
                        <span class="mode-option-name">Suggest</span>
                        <span class="mode-option-desc">proposes each change, you accept</span>
                    </button>
                    <button class="mode-option" id="mode-auto" data-action="setMode" data-a1="auto">
                        <span class="mode-option-dot"></span>
                        <span class="mode-option-name">Auto</span>
                        <span class="mode-option-desc">applies directly, gates are the net</span>
                    </button>
                    <button class="mode-option" id="mode-chat" data-action="setMode" data-a1="chat">
                        <span class="mode-option-dot"></span>
                        <span class="mode-option-name">Chat</span>
                        <span class="mode-option-desc">answers only, never writes</span>
                    </button>
                </div>
            </div>
            <button class="compose-clear" data-action="clearChat" title="Clear the conversation">Clear</button>
        </div>
    </div>

    <script nonce="${nonce}">
        ${js}
    </script>
</body>
</html>`;
}
