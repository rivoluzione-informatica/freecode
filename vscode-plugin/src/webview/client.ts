export function getWebviewJs(): string {
    return `
        const vscode = acquireVsCodeApi();

        // --- HTML/argument escaping (model-controlled strings must pass through these) ---
        function escapeHtml(s) {
            return String(s)
                .replace(/&/g, '&amp;')
                .replace(/</g, '&lt;')
                .replace(/>/g, '&gt;')
                .replace(/"/g, '&quot;')
                .replace(/'/g, '&#39;');
        }
        // NOTE: there used to be an encArg() here, to make a value safe to drop inside an
        // inline click handler. Handlers are now delegated (see fcDispatch at the bottom) and
        // every value travels in a data-* attribute instead, so nothing is ever concatenated into
        // executable JS. escapeHtml on the way in, el.dataset on the way out — the attack surface
        // it defended no longer exists.

        let currentMode = 'hitl';
        let activeStep = null;
        
        // Trajectory State
        let currentPrompt = '';
        let currentFilesRead = [];
        let currentProposals = [];
        let currentGates = [];
        let currentResponse = '';
        let currentOutcome = 'running';
        let currentSessionId = 'default';

        // Cumulative Cost State
        let sessionTokens = 0;
        let sessionLatency = 0;
        let sessionCalls = 0;
        let sessionSpent = 0;

        // Decision Cache
        let decisionCache = {};

        // RFC-001 Slice 2: tool_call id -> its timeline element (so tool_result updates it)
        let toolCallEls = {};

        // Timeline Groupings State
        let activeGroupedProposalItem = null;
        let accumulatedProposals = [];
        let activeGroupedGateItem = null;
        let accumulatedGates = [];
        let activeAssistantMessageBubble = null;
        let activeAssistantMessageText = "";
        let isGitPanelOpen = false;
        let isMemoryPanelOpen = false;
        let isProjectMemoryExpanded = true;
        let isGlobalMemoryExpanded = false;

        let activeTimeline = null;
        // PIC-10 — compact horizontal pipeline strip
        let fcMode = localStorage.getItem('fcMode') || 'compact'; // compact | vertical | hidden
        let fcStripEl = null;
        let fcState = null;
        let pendingProposals = [];

        // Harness Observability State
        let currentConfidence = 0.80;
        let activeTimer = null;
        let activeStartTime = null;
        let isRunActive = false;
        let actualAttemptsCount = 1;
        let runLatency = 0;
        let isHarnessPanelOpen = false;
        let isParamsOpen = false;

        // Auto-expand input box height based on content
        const chatInput = document.getElementById('chatInput');
        chatInput.addEventListener('input', function() {
            this.style.height = 'auto';
            this.style.height = (this.scrollHeight - 4) + 'px';
        });

        // Initial connection check
        vscode.postMessage({ type: 'checkConnection' });
        vscode.postMessage({ type: 'getMemories' });
        setInterval(() => {
            vscode.postMessage({ type: 'checkConnection' });
        }, 10000);

        const MODE_LABEL = { hitl: 'Suggest', auto: 'Auto', chat: 'Chat' };

        function setMode(mode) {
            const oldMode = currentMode;
            currentMode = mode;
            document.querySelectorAll('.mode-option').forEach(btn => btn.classList.remove('active'));
            const chosen = document.getElementById('mode-' + mode);
            if (chosen) chosen.classList.add('active');
            const label = document.getElementById('modeTriggerLabel');
            if (label) label.innerText = MODE_LABEL[mode] || mode;
            // The trigger carries the mode as a data attribute so CSS can tint it: Auto writes
            // without asking, and that must be visible without reading the word.
            const trigger = document.getElementById('modeTrigger');
            if (trigger) trigger.setAttribute('data-mode', mode);
            closeModeMenu();
            vscode.postMessage({ type: 'modeChanged', oldMode: oldMode, newMode: mode });
        }

        function closeModeMenu() {
            const menu = document.getElementById('modeMenu');
            if (menu) menu.classList.remove('open');
        }

        function toggleModeMenu() {
            const menu = document.getElementById('modeMenu');
            if (menu) menu.classList.toggle('open');
        }

        // --- Write scope -------------------------------------------------------
        // Static by design: it states what CAN happen. The only thing that ever moves here is a
        // row lighting up when that location is actually written during a turn.
        let scopeEntries = [];
        let scopeExclusions = [];

        function renderScope() {
            const summary = document.getElementById('scopeSummaryText');
            const detail = document.getElementById('scopeDetail');
            if (!summary || !detail) return;
            if (!scopeEntries.length) {
                summary.innerText = 'no folder open';
                detail.innerHTML = '';
                return;
            }
            const root = scopeEntries.find(e => e.label === 'workspace');
            const outside = scopeEntries.filter(e => !e.inside).length;
            const rootName = root ? root.path.split('/').filter(Boolean).pop() : '—';
            summary.innerHTML =
                'writes in <strong>' + escapeHtml(rootName) + '/</strong>' +
                (outside ? ' <span class="scope-outside-count">+' + outside + ' outside repo</span>' : '');

            let html = '';
            for (const e of scopeEntries) {
                html +=
                    '<div class="scope-row' + (e.inside ? '' : ' outside') + '" id="scope-row-' + escapeHtml(e.label.replace(/[^a-z]/gi, '')) + '">' +
                        '<span class="scope-dot"></span>' +
                        '<span class="scope-path" title="' + escapeHtml(e.path) + '">' + escapeHtml(shortenPath(e.path)) + '</span>' +
                        '<span class="scope-access ' + escapeHtml(e.access) + '">' + escapeHtml(e.access) + '</span>' +
                        '<span class="scope-note">' + escapeHtml(e.note) + '</span>' +
                    '</div>';
            }
            if (scopeExclusions.length) {
                html += '<div class="scope-row excluded">' +
                            '<span class="scope-dot"></span>' +
                            '<span class="scope-path">excluded from scans</span>' +
                            '<span class="scope-access ro">skip</span>' +
                            '<span class="scope-note">' + escapeHtml(scopeExclusions.join(', ')) + '</span>' +
                        '</div>';
            }
            detail.innerHTML = html;
        }

        function shortenPath(p) {
            if (!scopeHome) return p;
            return p.startsWith(scopeHome) ? '~' + p.slice(scopeHome.length) : p;
        }
        let scopeHome = '';

        function toggleScope() {
            const bar = document.getElementById('scopeBar');
            const chev = document.getElementById('scopeChevron');
            if (!bar) return;
            const open = bar.classList.toggle('open');
            if (chev) chev.innerText = open ? '▴' : '▾';
        }

        /// Flash the workspace row when a write actually lands this turn.
        function markScopeWritten() {
            const row = document.getElementById('scope-row-workspace');
            if (!row) return;
            row.classList.add('written');
            const bar = document.getElementById('scopeBar');
            if (bar) bar.classList.add('active-write');
        }

        function resetScopeActivity() {
            document.querySelectorAll('.scope-row.written').forEach(r => r.classList.remove('written'));
            const bar = document.getElementById('scopeBar');
            if (bar) bar.classList.remove('active-write');
        }

        // Resizer function for panels
        let startY, startHeight;
        let resizeTargetId;

        function initResize(e, targetId) {
            e.preventDefault();
            resizeTargetId = targetId;
            const targetEl = document.getElementById(targetId);
            startY = e.clientY;
            startHeight = parseInt(document.defaultView.getComputedStyle(targetEl).height, 10);
            document.documentElement.addEventListener('mousemove', doResize, false);
            document.documentElement.addEventListener('mouseup', stopResize, false);
        }

        function doResize(e) {
            const targetEl = document.getElementById(resizeTargetId);
            if (!targetEl) return;
            const newHeight = startHeight + (e.clientY - startY);
            if (newHeight > 40) {
                targetEl.style.height = newHeight + 'px';
                targetEl.style.maxHeight = newHeight + 'px';
            }
        }

        function stopResize(e) {
            document.documentElement.removeEventListener('mousemove', doResize, false);
            document.documentElement.removeEventListener('mouseup', stopResize, false);
        }

        function toggleAstSection() {
            const card = document.getElementById('astCard');
            const btn = document.getElementById('astBtn');
            const isOpen = card.classList.toggle('open');
            if (isOpen) {
                btn.classList.add('active');
                vscode.postMessage({ type: 'getActiveFilePath' });
            } else {
                btn.classList.remove('active');
            }
        }

        function toggleGitPanel() {
            const panel = document.getElementById('gitPanel');
            const btn = document.getElementById('gitToggleBtn');
            isGitPanelOpen = !isGitPanelOpen;
            if (isGitPanelOpen) {
                panel.style.display = 'block';
                btn.classList.add('active');
                refreshGitStatus();
            } else {
                panel.style.display = 'none';
                btn.classList.remove('active');
            }
        }

        function toggleMemoryPanel() {
            const panel = document.getElementById('memoryPanel');
            const btn = document.getElementById('memoryToggleBtn');
            isMemoryPanelOpen = !isMemoryPanelOpen;
            if (isMemoryPanelOpen) {
                panel.style.display = 'block';
                btn.classList.add('active');
                vscode.postMessage({ type: 'getMemories' });
            } else {
                panel.style.display = 'none';
                btn.classList.remove('active');
            }
        }

        function toggleMemorySubSection(type) {
            if (type === 'project') {
                isProjectMemoryExpanded = !isProjectMemoryExpanded;
                const content = document.getElementById('projectMemoryContent');
                const chevron = document.getElementById('projectMemoryChevron');
                content.style.display = isProjectMemoryExpanded ? 'flex' : 'none';
                chevron.innerText = isProjectMemoryExpanded ? '▼' : '▲';
            } else {
                isGlobalMemoryExpanded = !isGlobalMemoryExpanded;
                const content = document.getElementById('globalMemoryContent');
                const chevron = document.getElementById('globalMemoryChevron');
                content.style.display = isGlobalMemoryExpanded ? 'flex' : 'none';
                chevron.innerText = isGlobalMemoryExpanded ? '▼' : '▲';
            }
        }

        function addMemoryNote(type) {
            const input = document.getElementById(type === 'project' ? 'addProjectMemoryInput' : 'addGlobalMemoryInput');
            const content = input.value.trim();
            if (!content) return;
            const note = {
                id: 'mem_' + Date.now() + '_' + Math.random().toString(36).substring(2, 6),
                content: content
            };
            vscode.postMessage({
                type: 'saveMemory',
                memoryType: type,
                note: note
            });
            input.value = '';
        }

        function deleteMemoryNote(type, id) {
            vscode.postMessage({
                type: 'deleteMemory',
                memoryType: type,
                id: id
            });
        }

        // Note: double escape backslashes since it is inside a template string
        function startEditMemory(type, id) {
            document.getElementById('mem-display-' + id).style.display = 'none';
            document.getElementById('mem-edit-' + id).style.display = 'flex';
            document.getElementById('mem-input-' + id).focus();
        }

        function cancelEditMemory(id) {
            document.getElementById('mem-display-' + id).style.display = 'flex';
            document.getElementById('mem-edit-' + id).style.display = 'none';
        }

        function saveEditMemory(type, id) {
            const newContent = document.getElementById('mem-input-' + id).value.trim();
            if (!newContent) return;
            const note = {
                id: id,
                content: newContent
            };
            vscode.postMessage({
                type: 'saveMemory',
                memoryType: type,
                note: note
            });
        }

        function updateMemoryLists(project, global) {
            const projectList = document.getElementById('projectMemoryList');
            const globalList = document.getElementById('globalMemoryList');
            if (project.length === 0) {
                projectList.innerHTML = '<div style="color: var(--text-muted); font-size: 11px; padding: 4px 0;">No project memories.</div>';
            } else {
                projectList.innerHTML = project.map(note => 
                    '<div class="memory-item" style="display: flex; flex-direction: column; background: var(--card-bg); border: 1px solid var(--card-border); border-radius: 4px; padding: 4px 6px; gap: 2px; margin-bottom: 4px;">' +
                        '<div class="memory-item-display" id="mem-display-' + note.id + '" style="display: flex; justify-content: space-between; align-items: flex-start; gap: 6px;">' +
                            '<span class="memory-item-text" style="font-size: 11px; word-break: break-word; color: var(--text-main);">' + escapeHtml(note.content) + '</span>' +
                            '<div style="display: flex; gap: 4px; flex-shrink: 0; align-items: center; justify-content: center;">' +
                                '<span class="memory-item-action" title="Edit" data-action="startEditMemory" data-a1="project" data-a2="' + escapeHtml(note.id) + '" style="cursor: pointer; color: var(--text-muted); font-size: 10px; padding: 0 2px;">✎</span>' +
                                '<span class="memory-item-action" title="Delete" data-action="deleteMemoryNote" data-a1="project" data-a2="' + escapeHtml(note.id) + '" style="cursor: pointer; color: var(--vscode-errorForeground, #ef4444); font-size: 12px; line-height: 1; padding: 0 2px;">&times;</span>' +
                            '</div>' +
                        '</div>' +
                        '<div class="memory-item-edit" id="mem-edit-' + note.id + '" style="display: none; gap: 4px; align-items: center;">' +
                            '<input type="text" id="mem-input-' + note.id + '" class="input-text" style="margin-bottom: 0; padding: 2px 4px; font-size: 11px; flex-grow: 1;" value="' + escapeHtml(note.content) + '">' +
                            '<span class="memory-item-action" title="Save" data-action="saveEditMemory" data-a1="project" data-a2="' + escapeHtml(note.id) + '" style="cursor: pointer; color: var(--vscode-gitDecoration-addedResourceForeground, #10b981); font-size: 11px; font-weight: bold;">✓</span>' +
                            '<span class="memory-item-action" title="Cancel" data-action="cancelEditMemory" data-a1="' + escapeHtml(note.id) + '" style="cursor: pointer; color: var(--text-muted); font-size: 12px; font-weight: bold;">&times;</span>' +
                        '</div>' +
                    '</div>'
                ).join('');
            }
            if (global.length === 0) {
                globalList.innerHTML = '<div style="color: var(--text-muted); font-size: 11px; padding: 4px 0;">No global memories.</div>';
            } else {
                globalList.innerHTML = global.map(note => 
                    '<div class="memory-item" style="display: flex; flex-direction: column; background: var(--card-bg); border: 1px solid var(--card-border); border-radius: 4px; padding: 4px 6px; gap: 2px; margin-bottom: 4px;">' +
                        '<div class="memory-item-display" id="mem-display-' + note.id + '" style="display: flex; justify-content: space-between; align-items: flex-start; gap: 6px;">' +
                            '<span class="memory-item-text" style="font-size: 11px; word-break: break-word; color: var(--text-main);">' + escapeHtml(note.content) + '</span>' +
                            '<div style="display: flex; gap: 4px; flex-shrink: 0; align-items: center; justify-content: center;">' +
                                '<span class="memory-item-action" title="Edit" data-action="startEditMemory" data-a1="global" data-a2="' + escapeHtml(note.id) + '" style="cursor: pointer; color: var(--text-muted); font-size: 10px; padding: 0 2px;">✎</span>' +
                                '<span class="memory-item-action" title="Delete" data-action="deleteMemoryNote" data-a1="global" data-a2="' + escapeHtml(note.id) + '" style="cursor: pointer; color: var(--vscode-errorForeground, #ef4444); font-size: 12px; line-height: 1; padding: 0 2px;">&times;</span>' +
                            '</div>' +
                        '</div>' +
                        '<div class="memory-item-edit" id="mem-edit-' + note.id + '" style="display: none; gap: 4px; align-items: center;">' +
                            '<input type="text" id="mem-input-' + note.id + '" class="input-text" style="margin-bottom: 0; padding: 2px 4px; font-size: 11px; flex-grow: 1;" value="' + escapeHtml(note.content) + '">' +
                            '<span class="memory-item-action" title="Save" data-action="saveEditMemory" data-a1="global" data-a2="' + escapeHtml(note.id) + '" style="cursor: pointer; color: var(--vscode-gitDecoration-addedResourceForeground, #10b981); font-size: 11px; font-weight: bold;">✓</span>' +
                            '<span class="memory-item-action" title="Cancel" data-action="cancelEditMemory" data-a1="' + escapeHtml(note.id) + '" style="cursor: pointer; color: var(--text-muted); font-size: 12px; font-weight: bold;">&times;</span>' +
                        '</div>' +
                    '</div>'
                ).join('');
            }
        }

        function toggleHarnessPanel() {
            const panel = document.getElementById('harnessPanel');
            const btn = document.getElementById('harnessToggleBtn');
            isHarnessPanelOpen = !isHarnessPanelOpen;
            if (isHarnessPanelOpen) {
                panel.style.display = 'block';
                btn.classList.add('active');
            } else {
                panel.style.display = 'none';
                btn.classList.remove('active');
            }
        }

        let isSettingsOpen = false;

        function toggleSettingsPanel() {
            const panel = document.getElementById('settingsPanel');
            const btn = document.getElementById('settingsToggleBtn');
            isSettingsOpen = !isSettingsOpen;
            if (isSettingsOpen) {
                panel.style.display = 'block';
                btn.classList.add('active');
                
                 // Pre-fill from localStorage
                 document.getElementById('settingLlmEndpoint').value = localStorage.getItem('llmEndpoint') || 'http://127.0.0.1:1234/v1/chat/completions';
                 document.getElementById('settingLlmModel').value = localStorage.getItem('llmModel') || 'gemma-4-e2b-it-mlx';
                 document.getElementById('settingExcludedFiles').value = localStorage.getItem('excludedFiles') || '';
                 
                 const elMono = document.getElementById('settingMonotonicity');
                 if (elMono) elMono.checked = localStorage.getItem('monotonicityFilter') === 'true';
             } else {
                 panel.style.display = 'none';
                 btn.classList.remove('active');
             }
         }
 
         function saveSettings() {
             const endpointInput = document.getElementById('settingLlmEndpoint');
             const modelInput = document.getElementById('settingLlmModel');
             const excludedInput = document.getElementById('settingExcludedFiles');
             const monotonicityInput = document.getElementById('settingMonotonicity');
             
             let endpoint = endpointInput.value.trim();
             let model = modelInput.value.trim();
             let excluded = excludedInput.value.trim();
             let monotonicity = monotonicityInput ? monotonicityInput.checked : false;
             
             if (!endpoint) endpoint = 'http://127.0.0.1:1234/v1/chat/completions';
             if (!model) model = 'gemma-4-e2b-it-mlx';
             
             localStorage.setItem('llmEndpoint', endpoint);
             localStorage.setItem('llmModel', model);
             localStorage.setItem('excludedFiles', excluded);
             localStorage.setItem('monotonicityFilter', monotonicity ? 'true' : 'false');
             
             endpointInput.value = endpoint;
             modelInput.value = model;
             excludedInput.value = excluded;
             
             const excludedArray = excluded.split(',').map(s => s.trim()).filter(Boolean);
             
             vscode.postMessage({
                 type: 'writeConfig',
                 config: {
                     excluded_files: excludedArray,
                     llm_endpoint: endpoint,
                     llm_model: model,
                     mode: currentMode,
                     monotonicity_filter: monotonicity
                 }
             });

             updateActiveScopeBanner(excludedArray);
             
             toggleSettingsPanel();
             
             appendMessage('assistant', 'Settings updated! LLM Endpoint: ' + endpoint + ', Model: ' + model + (excludedArray.length ? ', Exclusions: ' + excludedArray.join(', ') : '') + ', Monotonicity: ' + (monotonicity ? 'On' : 'Off'));
         }

        function clearChat() {
            const chatContainer = document.getElementById('chatContainer');
            chatContainer.innerHTML = '<div class="message assistant"><div class="message-sender">FreeCode</div><div class="message-bubble">Chat history cleared. How can I help you?</div></div>';
            // New session → reset the cumulative harness counters so they don't carry over.
            sessionTokens = 0; sessionLatency = 0; sessionCalls = 0; sessionSpent = 0;
            const z = { valSessionTokens: '0', valSessionLatency: '0.0s', valSessionCalls: '0', valSessionSpent: '€0.00' };
            Object.keys(z).forEach(id => { const el = document.getElementById(id); if (el) el.innerText = z[id]; });
            vscode.postMessage({ type: 'clearChat' });
        }

        function pingDaemon() {
            const badge = document.getElementById('statusBadge');
            const text = document.getElementById('statusText');
            if (badge) badge.className = 'status-badge checking';
            if (text) text.innerText = 'Checking…';
            vscode.postMessage({ type: 'checkConnection' });
        }

        function refreshGitStatus() {
            vscode.postMessage({ type: 'getGitStatus' });
        }

        // Fetch git status on load and periodically
        setTimeout(refreshGitStatus, 1000);
        setInterval(refreshGitStatus, 10000);

        function updateGitPanel(status) {
            const title = document.getElementById('gitPanelTitle');
            const list = document.getElementById('gitFileList');
            
            if (!status.is_inside_repo) {
                title.innerText = 'Git: Not a repository';
                list.innerHTML = '<div style="color: var(--text-muted); padding: 4px 0;">Not inside a Git repository.</div>';
                return;
            }

            const totalChanges = status.modified_files.length + status.added_files.length + status.deleted_files.length;
            title.innerText = 'Git: ' + status.branch + ' (' + totalChanges + ' changes)';

            if (totalChanges === 0) {
                list.innerHTML = '<div style="color: var(--text-muted); padding: 4px 0;">No uncommitted changes.</div>';
                return;
            }

            let html = '';
            status.modified_files.forEach(f => {
                html += '<div class="git-file-item">' +
                        '<span class="git-file-status modified">M</span>' +
                        '<span class="git-file-name" data-action="openFile" data-a1="' + escapeHtml(f) + '">' + escapeHtml(f) + '</span>' +
                        '<div class="git-file-actions">' +
                        '<span class="git-diff-btn" title="Open Diff" data-action="openDiff" data-stop="1" data-a1="' + escapeHtml(f) + '">Δ Diff</span>' +
                        '</div>' +
                        '</div>';
            });
            status.added_files.forEach(f => {
                html += '<div class="git-file-item">' +
                        '<span class="git-file-status added">A</span>' +
                        '<span class="git-file-name" data-action="openFile" data-a1="' + escapeHtml(f) + '">' + escapeHtml(f) + '</span>' +
                        '<div class="git-file-actions">' +
                        '<span class="git-diff-btn" title="Open Diff" data-action="openDiff" data-stop="1" data-a1="' + escapeHtml(f) + '">Δ Diff</span>' +
                        '</div>' +
                        '</div>';
            });
            status.deleted_files.forEach(f => {
                html += '<div class="git-file-item">' +
                        '<span class="git-file-status deleted">D</span>' +
                        '<span class="git-file-name" style="color: var(--text-muted); cursor: default;">' + escapeHtml(f) + '</span>' +
                        '</div>';
            });
            list.innerHTML = html;
        }

        function openDiff(filePath) {
            // The path now arrives raw from a data-* attribute (it used to be encArg-encoded),
            // so there is nothing to decode - and decoding here would corrupt any path
            // legitimately containing a '%'.
            vscode.postMessage({ type: 'openDiff', filePath: filePath });
        }

        function parseMarkdown(text) {
            // Escape HTML
            let html = text
                .replace(/&/g, "&amp;")
                .replace(/</g, "&lt;")
                .replace(/>/g, "&gt;");

            // Code blocks
            html = html.replace(/\\x60\\x60\\x60(\\w*)\\n([\\s\\S]*?)\\n\\x60\\x60\\x60/g, function(match, lang, code) {
                const isCompilerError = code.includes('error[E') || code.includes('error:') || code.includes('compiler errors') || code.includes('cargo check');
                if (isCompilerError) {
                    return '<details class="compiler-error-details" open><summary>Compiler Output (Click to Collapse)</summary><pre><code class="language-' + lang + '">' + code.trim() + '</code></pre></details>';
                }
                return '<pre><code class="language-' + lang + '">' + code.trim() + '</code></pre>';
            });

            // Inline code and file link auto-detection
            html = html.replace(/\\x60([^\\x60\\n]+)\\x60/g, function(match, codeText) {
                const isFilePath = /^[a-zA-Z0-9_\\-\\/]+\\.[a-zA-Z0-9]+$/.test(codeText);
                if (isFilePath) {
                    return '<code class="file-link" data-action="openFile" data-a1="' + escapeHtml(codeText) + '">' + escapeHtml(codeText) + '</code>';
                }
                return '<code>' + codeText + '</code>';
            });

            // Markdown link parsing
            html = html.replace(/\\\[([^\\\]]+)\\\]\\\(([^)]+)\\\)/g, function(match, label, url) {
                if (url.startsWith('http://') || url.startsWith('https://')) {
                    return '<a href="' + url.replace(/"/g, '&quot;') + '" target="_blank">' + label + '</a>';
                } else {
                    let filePath = url;
                    if (filePath.startsWith('file://')) {
                        filePath = filePath.substring(7);
                    }
                    return '<a href="#" data-action="openFile" data-a1="' + escapeHtml(filePath) + '">' + label + '</a>';
                }
            });

            // Bold
            html = html.replace(/\\*\\*([^*]+)\\*\\*/g, '<strong>$1</strong>');

            // Bullet Lists
            html = html.replace(/^(?:\\-|\\*)\\s+(.+)$/gm, '<li class="ul-item">$1</li>');
            html = html.replace(/((?:<li class="ul-item">.*<\\/li>\\s*)+)/g, '<ul>$1</ul>');

            // Numbered Lists
            html = html.replace(/^\\d+\\.\\s+(.+)$/gm, '<li class="ol-item">$1</li>');
            html = html.replace(/((?:<li class="ol-item">.*<\\/li>\\s*)+)/g, '<ol>$1</ol>');

            // Headers
            html = html.replace(/^###\\s+(.+)$/gm, '<h3>$1</h3>');
            html = html.replace(/^##\\s+(.+)$/gm, '<h2>$1</h2>');
            html = html.replace(/^#\\s+(.+)$/gm, '<h1>$1</h1>');

            // Horizontal rules
            html = html.replace(/^---$/gm, '<hr>');

            // Newlines to breaks
            html = html.replace(/\\n\\n/g, '<br><br>');

            return html;
        }

        function appendMessage(sender, text) {
            const chatContainer = document.getElementById('chatContainer');
            const msgDiv = document.createElement('div');
            msgDiv.className = 'message ' + sender;

            const senderDiv = document.createElement('div');
            senderDiv.className = 'message-sender';
            senderDiv.innerText = sender === 'user' ? 'You' : 'FreeCode';
            msgDiv.appendChild(senderDiv);

            const bubbleDiv = document.createElement('div');
            bubbleDiv.className = 'message-bubble';
            if (sender === 'user') {
                bubbleDiv.innerText = text;
            } else {
                bubbleDiv.innerHTML = parseMarkdown(text);
            }
            msgDiv.appendChild(bubbleDiv);

            chatContainer.appendChild(msgDiv);
            chatContainer.scrollTop = chatContainer.scrollHeight;
        }

        function createAssistantPlaceholder() {
            const chatContainer = document.getElementById('chatContainer');
            const msgDiv = document.createElement('div');
            msgDiv.className = 'message assistant';

            const senderDiv = document.createElement('div');
            senderDiv.className = 'message-sender';
            senderDiv.innerText = 'FreeCode';
            msgDiv.appendChild(senderDiv);

            const bubbleDiv = document.createElement('div');
            bubbleDiv.className = 'message-bubble';
            bubbleDiv.innerHTML = '<span class="spinner"></span> <em>FreeCode is thinking...</em>';
            msgDiv.appendChild(bubbleDiv);

            chatContainer.appendChild(msgDiv);
            chatContainer.scrollTop = chatContainer.scrollHeight;
            
            activeAssistantMessageBubble = bubbleDiv;
            activeAssistantMessageText = "";
        }

        function openFile(filePath) {
            // The path now arrives raw from a data-* attribute (it used to be encArg-encoded),
            // so there is nothing to decode - and decoding here would corrupt any path
            // legitimately containing a '%'.
            vscode.postMessage({ type: 'openFile', filePath: filePath });
        }

        function computeDiff(oldContent, newContent) {
            const oldLines = oldContent.split('\\n');
            const newLines = newContent.split('\\n');
            const n = oldLines.length;
            const m = newLines.length;
            
            const dp = Array(n + 1).fill(null).map(() => Array(m + 1).fill(0));
            for (let i = 1; i <= n; i++) {
                for (let j = 1; j <= m; j++) {
                    if (oldLines[i - 1] === newLines[j - 1]) {
                        dp[i][j] = dp[i - 1][j - 1] + 1;
                    } else {
                        dp[i][j] = Math.max(dp[i - 1][j], dp[i][j - 1]);
                    }
                }
            }
            
            const diff = [];
            let i = n;
            let j = m;
            while (i > 0 || j > 0) {
                if (i > 0 && j > 0 && oldLines[i - 1] === newLines[j - 1]) {
                    diff.unshift({ type: 'unchanged', text: oldLines[i - 1] });
                    i--;
                    j--;
                } else if (j > 0 && (i === 0 || dp[i][j - 1] >= dp[i - 1][j])) {
                    diff.unshift({ type: 'added', text: newLines[j - 1] });
                    j--;
                } else {
                    diff.unshift({ type: 'removed', text: oldLines[i - 1] });
                    i--;
                }
            }
            return diff;
        }

        // PIC-10 — compact horizontal pipeline strip. Additive: the verbose timeline still renders
        // (hidden by CSS in compact mode); the strip mirrors the same accumulated state on one line.
        function fcReset() {
            fcState = { ctx: null, gp: 0, gt: 0, gfail: false, greason: '', at: null, am: null, lat: null, phase: 'intent', done: false };
        }
        function fcSyncGates() {
            if (!fcState) return;
            fcState.gt = accumulatedGates.length;
            fcState.gp = accumulatedGates.filter(g => g.passed).length;
            const failed = accumulatedGates.find(g => !g.passed);
            fcState.gfail = !!failed;
            fcState.greason = failed ? (failed.gateName + (failed.reasons && failed.reasons[0] ? ': ' + failed.reasons[0] : '')) : '';
            fcState.phase = 'gates';
        }
        function fcChip(label, cls) { return '<span class="fc-chip ' + cls + '">' + label + '</span>'; }
        function renderFcStrip() {
            if (!fcStripEl || !fcState) return;
            const s = fcState;
            const ln = '<span class="fc-ln"></span>';
            if (s.done) {
                const gatesLbl = s.gt > 0 ? ('gates ' + s.gp + '/' + s.gt) : 'gates —';
                const bits = [];
                if (s.ctx != null) bits.push('ctx ' + s.ctx);
                if (s.at != null) bits.push(s.at + ' turn' + (s.at > 1 ? 's' : ''));
                if (s.lat != null && s.lat > 0) bits.push(s.lat.toFixed(1) + 's');
                const meta = bits.length ? bits.join(' · ') : 'done';
                let html = '<div class="fc-pipe">' + fcChip(gatesLbl, s.gfail ? 'fc-bad' : 'fc-gate') +
                    '<span class="fc-meta">' + meta + '</span>' +
                    '<button class="fc-exp" data-action="fcExpand">details ▾</button></div>';
                if (s.gfail && s.greason) html += '<div class="fc-why">⚠ ' + escapeHtml(s.greason) + '</div>';
                fcStripEl.innerHTML = html;
                return;
            }
            const act = (p) => (s.phase === p) ? ' fc-active' : '';
            const parts = [
                fcChip(s.phase === 'intent' ? 'intent' : '✓ intent', s.phase === 'intent' ? 'fc-active' : 'fc-done'), ln,
                fcChip(s.ctx != null ? '✓ ctx ' + s.ctx : 'ctx', s.ctx != null ? 'fc-done' : 'fc-pend' + act('ctx')), ln,
                (s.gt > 0 ? fcChip('gates ' + s.gp + '/' + s.gt, s.gfail ? 'fc-bad' : 'fc-gate') : fcChip('gates', 'fc-pend' + act('gates'))), ln,
                (s.at != null ? fcChip('agent ' + s.at + '/' + s.am, 'fc-active') : fcChip('agent', 'fc-pend' + act('agent'))), ln,
                fcChip('done', 'fc-pend' + act('done')),
            ];
            let html = '<div class="fc-pipe">' + parts.join('') + '</div>';
            if (s.gfail && s.greason) html += '<div class="fc-why">⚠ ' + escapeHtml(s.greason) + '</div>';
            fcStripEl.innerHTML = html;
        }
        function fcCycleMode() {
            fcMode = fcMode === 'compact' ? 'vertical' : (fcMode === 'vertical' ? 'hidden' : 'compact');
            localStorage.setItem('fcMode', fcMode);
            document.querySelectorAll('.timeline-container').forEach(c => {
                c.classList.remove('fc-compact', 'fc-vertical', 'fc-hidden', 'fc-expanded');
                c.classList.add('fc-' + fcMode);
            });
            const b = document.getElementById('fcStripBtn');
            if (b) b.title = 'Pipeline view: ' + fcMode + ' (click to cycle)';
        }
        function fcExpand(btn) {
            const c = btn.closest('.timeline-container');
            if (!c) return;
            const exp = c.classList.toggle('fc-expanded');
            btn.textContent = exp ? 'details ▴' : 'details ▾';
        }

        function getOrCreateTimeline() {
            if (!activeTimeline) {
                const chatContainer = document.getElementById('chatContainer');
                activeTimeline = document.createElement('div');
                activeTimeline.className = 'timeline-container fc-' + fcMode;
                fcStripEl = document.createElement('div');
                fcStripEl.className = 'fc-strip';
                activeTimeline.appendChild(fcStripEl);
                fcReset();
                renderFcStrip();
                chatContainer.appendChild(activeTimeline);
                chatContainer.scrollTop = chatContainer.scrollHeight;
            }
            return activeTimeline;
        }

        function addTimelineItem(status, title, detailsHtml) {
            const timeline = getOrCreateTimeline();
            const item = document.createElement('div');
            item.className = 'timeline-item';

            let icon = '•';
            let badgeClass = status; 
            if (status === 'success') icon = '✓';
            else if (status === 'error') icon = '✗';
            else if (status === 'running') {
                icon = '⟳';
                badgeClass = 'running';
            } else if (status === 'proposal') {
                icon = 'Δ';
            }

            item.innerHTML = 
                '<div class="timeline-badge ' + badgeClass + '">' + icon + '</div>' +
                '<div class="timeline-content">' +
                    '<div class="timeline-header">' +
                        '<span class="timeline-title-text">' + escapeHtml(title) + '</span>' +
                    '</div>' +
                    (detailsHtml ? '<div class="timeline-details">' + detailsHtml + '</div>' : '') +
                '</div>';
            timeline.appendChild(item);
            const chatContainer = document.getElementById('chatContainer');
            chatContainer.scrollTop = chatContainer.scrollHeight;
            return item;
        }

        // Resolve the currently-running timeline item (kills its spinner) when the turn moves on
        // or ends — otherwise a fatal error leaves the prior step spinning forever.
        function resolveActiveStep() {
            if (activeStep) {
                const badge = activeStep.querySelector('.timeline-badge');
                if (badge && badge.classList.contains('running')) {
                    badge.classList.remove('running');
                    badge.classList.add('success');
                    badge.textContent = '✓';
                }
                activeStep = null;
            }
        }

        function appendStep(status, text) {
            // Deprecated in favor of addTimelineItem, but kept for compatibility/re-use
            return addTimelineItem(status === 'info' ? 'running' : status, text);
        }

        function startHarnessTimer() {
            if (activeTimer) {
                clearInterval(activeTimer);
            }
            isRunActive = true;
            runLatency = 0;
            actualAttemptsCount = 1;
            activeStartTime = Date.now();

            const badge = document.getElementById('harnessModeBadge');
            if (badge) {
                badge.innerText = 'Active';
                badge.style.background = 'color-mix(in srgb, var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d) 15%, transparent)';
                badge.style.color = 'var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d)';
                badge.style.borderColor = 'color-mix(in srgb, var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d) 30%, transparent)';
            }

            const V = parseFloat(document.getElementById('paramV').value) || 0;
            const L = parseFloat(document.getElementById('paramL').value) || 0;
            const kappa = parseFloat(document.getElementById('paramKappa').value) || 0;
            const C_gen = parseFloat(document.getElementById('paramCgen').value) || 0;
            const C_ver = parseFloat(document.getElementById('paramCver').value) || 0;
            const R = parseInt(document.getElementById('paramR').value) || 3;
            currentConfidence = parseFloat(document.getElementById('paramG').value) || 0.80;

            document.getElementById('valConfidence').innerText = (currentConfidence * 100).toFixed(1) + '%';
            document.getElementById('valAttempts').innerText = '1 / ' + (R + 1);

            activeTimer = setInterval(() => {
                runLatency = (Date.now() - activeStartTime) / 1000;
                
                const liveCost = actualAttemptsCount * (C_gen + C_ver) + kappa * runLatency;
                document.getElementById('valCost').innerText = '€' + liveCost.toFixed(2);
                
                const livePayoff = (currentConfidence * V) - liveCost;
                document.getElementById('valPayoff').innerText = '€' + livePayoff.toFixed(2);
                stylePayoff('valPayoff', livePayoff);
                
                document.getElementById('valAttempts').innerText = actualAttemptsCount + ' (' + runLatency.toFixed(1) + 's)';
            }, 100);
        }

        function sendIntent() {
            const prompt = chatInput.value.trim();
            if (!prompt) {
                return;
            }
            resetScopeActivity();
            
            chatInput.value = '';
            chatInput.style.height = '18px';
            
            document.getElementById('sendBtn').style.display = 'none';
            document.getElementById('stopBtn').style.display = 'flex';
            
            appendMessage('user', prompt);
            
            // Reset streaming assistant bubble state
            activeAssistantMessageBubble = null;
            activeAssistantMessageText = "";
            
            // Reset Trajectory & Grouping State
            currentPrompt = prompt;
            currentFilesRead = [];
            currentProposals = [];
            currentGates = [];
            currentResponse = '';
            currentOutcome = 'running';
            activeGroupedProposalItem = null;
            accumulatedProposals = [];
            toolCallEls = {};
            activeGroupedGateItem = null;
            accumulatedGates = [];
            
            // Start the Harness timer & live tracking
            startHarnessTimer();
            
            const llmEndpoint = localStorage.getItem('llmEndpoint') || 'http://127.0.0.1:1234/v1/chat/completions';
            const llmModel = localStorage.getItem('llmModel') || 'gemma-4-e2b-it-mlx';
            
            vscode.postMessage({
                type: 'dispatch',
                prompt: prompt,
                mode: currentMode,
                llmEndpoint: llmEndpoint,
                llmModel: llmModel
            });
        }

        function stopIntent() {
            appendStep('error', 'Stopping agent execution...');
            vscode.postMessage({ type: 'stop' });
            
            document.getElementById('sendBtn').style.display = 'flex';
            document.getElementById('sendBtn').disabled = false;
            document.getElementById('stopBtn').style.display = 'none';
        }

        function applyAstEdit() {
            const filePath = document.getElementById('filePathInput').value.trim();
            const symbolName = document.getElementById('symbolInput').value.trim();
            const newContent = document.getElementById('astContentInput').value;

            if (!filePath || !symbolName) {
                appendStep('error', 'File Path and Symbol Name are required for AST edits.');
                return;
            }

            document.getElementById('astSpinner').style.display = 'inline-block';
            document.getElementById('applyAstBtn').disabled = true;

            vscode.postMessage({
                type: 'applyAst',
                filePath,
                symbolName,
                newContent
            });
        }

        function handleKeyPress(event) {
            if (event.key === 'Enter' && !event.shiftKey) {
                event.preventDefault();
                sendIntent();
            }
        }

        async function sha256(message) {
            const msgBuffer = new TextEncoder().encode(message);
            const hashBuffer = await crypto.subtle.digest('SHA-256', msgBuffer);
            const hashArray = Array.from(new Uint8Array(hashBuffer));
            return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
        }

        // The exclusion patterns used to be a cramped header banner; they are now a row of the
        // scope bar, next to everything else that defines the blast radius.
        function updateActiveScopeBanner(patterns) {
            scopeExclusions = patterns || [];
            renderScope();
        }

        async function updateGroupedProposals() {
            const resolvedProposals = [];
            let allCachedAccept = true;
            let anyCachedDiscard = false;
            let hasHitl = false;
            
            for (const prop of accumulatedProposals) {
                const hash = await sha256(prop.newContent);
                const cacheKey = prop.filePath + ":" + hash;
                const cached = decisionCache[cacheKey];
                
                resolvedProposals.push({
                    ...prop,
                    hash,
                    cached
                });
                
                if (prop.mode === 'hitl') {
                    hasHitl = true;
                    if (cached === 'accept') {
                        // cached accept
                    } else if (cached === 'discard') {
                        anyCachedDiscard = true;
                        allCachedAccept = false;
                    } else {
                        allCachedAccept = false;
                    }
                }
            }
            
            const count = resolvedProposals.length;
            const title = 'Proposed Changes (' + count + ' ' + (count === 1 ? 'file' : 'files') + ')';
            
            let filesHtml = '';
            const groupId = 'group_prop_' + Date.now();
            
            for (let i = 0; i < resolvedProposals.length; i++) {
                const p = resolvedProposals[i];
                const diff = computeDiff(p.oldContent, p.newContent);
                const diffLinesHtml = diff.map(line => {
                    let cls = 'unchanged';
                    let prefix = ' ';
                    if (line.type === 'added') { cls = 'added'; prefix = '+'; }
                    else if (line.type === 'removed') { cls = 'removed'; prefix = '-'; }
                    const escapedText = line.text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
                    return '<div class="diff-line ' + cls + '">' + prefix + ' ' + escapedText + '</div>';
                }).join('');
                
                let badgeHtml = '';
                if (p.cached) {
                    const badgeClass = p.cached === 'accept' ? 'applied' : 'discarded';
                    const badgeText = p.cached === 'accept' ? 'Auto-Approved (Cached)' : 'Auto-Discarded (Cached)';
                    badgeHtml = '<span class="proposal-status-badge ' + badgeClass + '" style="margin-left: 8px;">' + badgeText + '</span>';
                }

                let toggleEditHtml = '';
                if (hasHitl && !p.cached) {
                    toggleEditHtml = '<span class="edit-mode-toggle" data-action="toggleEditView" data-a1="' + escapeHtml(groupId) + '" data-a2="' + i + '">📝 Edit Code</span>';
                }
                
                filesHtml += 
                    '<div style="margin-bottom: 8px; border-bottom: 1px dashed var(--card-border); padding-bottom: 6px;">' +
                        '<div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 4px;">' +
                            '<code class="file-link" data-action="openFile" data-a1="' + escapeHtml(p.filePath) + '">' + escapeHtml(p.filePath) + '</code>' +
                            badgeHtml +
                        '</div>' +
                        '<div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px;">' +
                            '<span style="font-size: 10px; color: var(--text-muted);">Proposed modification</span>' +
                            toggleEditHtml +
                        '</div>' +
                        '<div id="view-diff-' + groupId + '-' + i + '">' +
                            '<details open>' +
                                '<summary style="cursor: pointer; font-size: 10px; color: var(--text-muted); outline: none;">Preview Code Diff</summary>' +
                                '<div class="diff-wrapper">' + diffLinesHtml + '</div>' +
                            '</details>' +
                        '</div>' +
                        '<div id="view-edit-' + groupId + '-' + i + '" style="display: none;">' +
                            '<textarea class="proposal-edit-textarea" id="textarea-' + groupId + '-' + i + '" data-input="updateProposedCode" data-a1="' + escapeHtml(groupId) + '" data-a2="' + i + '">' + p.newContent.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;") + '</textarea>' +
                        '</div>' +
                    '</div>';
            }
            
            let actionsHtml = '';
            if (hasHitl) {
                if (allCachedAccept) {
                    actionsHtml = '<div class="proposal-actions"><span class="proposal-status-badge applied">Auto-Approved (Cached)</span></div>';
                } else if (anyCachedDiscard) {
                    actionsHtml = '<div class="proposal-actions"><span class="proposal-status-badge discarded">Auto-Discarded (Cached)</span></div>';
                } else {
                    actionsHtml = 
                        '<div class="proposal-actions" id="actions-' + groupId + '">' +
                            '<button class="proposal-btn accept" data-action="submitGroupedHitlDecision" data-a1="accept" data-a2="' + escapeHtml(groupId) + '">Accept All Changes</button>' +
                            '<button class="proposal-btn discard" data-action="submitGroupedHitlDecision" data-a1="discard" data-a2="' + escapeHtml(groupId) + '">Discard All Changes</button>' +
                        '</div>';
                }
            } else {
                actionsHtml = 
                    '<div class="proposal-actions">' +
                        '<span class="proposal-status-badge applied">Executed (Auto)</span>' +
                    '</div>';
            }
            
            const detailsHtml = 
                '<div class="proposal-status" style="margin-bottom: 4px;">' +
                    '<span class="proposal-status-badge pending" id="badge-' + groupId + '">Proposed changes</span>' +
                '</div>' +
                filesHtml +
                actionsHtml;
                
            if (!activeGroupedProposalItem) {
                activeGroupedProposalItem = addTimelineItem('proposal', title, detailsHtml);
            } else {
                const titleEl = activeGroupedProposalItem.querySelector('.timeline-title-text');
                if (titleEl) titleEl.innerText = title;
                const detailsEl = activeGroupedProposalItem.querySelector('.timeline-details');
                if (detailsEl) detailsEl.innerHTML = detailsHtml;
            }
            
            pendingProposals = resolvedProposals.map(p => ({
                id: groupId,
                filePath: p.filePath,
                contentHash: p.hash,
                editedContent: p.newContent,
                element: activeGroupedProposalItem
            }));

            // If all are cached in HITL mode, we can auto-decide:
            if (hasHitl && resolvedProposals.length > 0) {
                if (anyCachedDiscard) {
                    console.log('Grouped proposals: Auto-discarding due to cached decision');
                    submitGroupedHitlDecision('discard', groupId);
                } else if (allCachedAccept) {
                    console.log('Grouped proposals: Auto-accepting due to cached decisions');
                    submitGroupedHitlDecision('accept', groupId);
                }
            }
        }
        
        function submitGroupedHitlDecision(decision, groupId) {
            const edits = {};
            pendingProposals.forEach(prop => {
                const cacheKey = prop.filePath + ":" + prop.contentHash;
                decisionCache[cacheKey] = decision;
                if (decision === 'accept' && prop.editedContent !== undefined && prop.editedContent !== null) {
                    edits[prop.filePath] = prop.editedContent;
                }
            });
            
            const container = document.getElementById('actions-' + groupId);
            if (container) {
                container.innerHTML = '<span style="font-size: 11px; color: var(--text-muted);">Sending decision: <strong>' + decision + '</strong>...</span>';
            }
            vscode.postMessage({
                type: 'hitlResponse',
                decision: decision,
                edits: edits
            });
        }

        function toggleEditView(btnEl, groupId, index) {
            const diffDiv = document.getElementById('view-diff-' + groupId + '-' + index);
            const editDiv = document.getElementById('view-edit-' + groupId + '-' + index);
            const toggleBtn = btnEl;
            if (diffDiv.style.display === 'none') {
                const p = accumulatedProposals[index];
                const textarea = document.getElementById('textarea-' + groupId + '-' + index);
                if (p && textarea) {
                    p.newContent = textarea.value;
                    const diff = computeDiff(p.oldContent, p.newContent);
                    const diffLinesHtml = diff.map(line => {
                        let cls = 'unchanged';
                        let prefix = ' ';
                        if (line.type === 'added') { cls = 'added'; prefix = '+'; }
                        else if (line.type === 'removed') { cls = 'removed'; prefix = '-'; }
                        const escapedText = line.text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
                        return '<div class="diff-line ' + cls + '">' + prefix + ' ' + escapedText + '</div>';
                    }).join('');
                    const wrapper = diffDiv.querySelector('.diff-wrapper');
                    if (wrapper) {
                        wrapper.innerHTML = diffLinesHtml;
                    }
                }
                
                diffDiv.style.display = 'block';
                editDiv.style.display = 'none';
                toggleBtn.innerText = '📝 Edit Code';
            } else {
                diffDiv.style.display = 'none';
                editDiv.style.display = 'block';
                toggleBtn.innerText = '🔍 Show Diff';
            }
        }
        
        function updateProposedCode(groupId, index) {
            const textarea = document.getElementById('textarea-' + groupId + '-' + index);
            if (textarea && accumulatedProposals[index]) {
                accumulatedProposals[index].newContent = textarea.value;
                const prop = pendingProposals.find(p => p.filePath === accumulatedProposals[index].filePath);
                if (prop) {
                    prop.editedContent = textarea.value;
                }
            }
        }

        function sendIntentWithPrompt(customPrompt) {
            document.getElementById('sendBtn').style.display = 'none';
            document.getElementById('stopBtn').style.display = 'flex';
            
            appendMessage('user', customPrompt);
            
            // Reset streaming assistant bubble state
            activeAssistantMessageBubble = null;
            activeAssistantMessageText = "";
            
            // Reset Trajectory & Grouping State
            currentPrompt = customPrompt;
            currentFilesRead = [];
            currentProposals = [];
            currentGates = [];
            currentResponse = '';
            currentOutcome = 'running';
            activeGroupedProposalItem = null;
            accumulatedProposals = [];
            toolCallEls = {};
            activeGroupedGateItem = null;
            accumulatedGates = [];
            
            // Start the Harness timer & live tracking
            startHarnessTimer();
            
            const llmEndpoint = localStorage.getItem('llmEndpoint') || 'http://127.0.0.1:1234/v1/chat/completions';
            const llmModel = localStorage.getItem('llmModel') || 'gemma-4-e2b-it-mlx';
            
            vscode.postMessage({
                type: 'dispatch',
                prompt: customPrompt,
                mode: currentMode,
                llmEndpoint: llmEndpoint,
                llmModel: llmModel
            });
        }

        function submitRecoveryRetry(recoveryId) {
            const inputEl = document.getElementById('input-' + recoveryId);
            const btnEl = document.getElementById('btn-' + recoveryId);
            if (!inputEl) return;
            const hint = inputEl.value.trim();
            if (!hint) return;
            
            inputEl.disabled = true;
            if (btnEl) btnEl.disabled = true;
            
            const customPrompt = "The compilation failed with errors. Operator corrected: " + hint + ". Please update the files.";
            sendIntentWithPrompt(customPrompt);
        }

        function retryLastPrompt() {
            // Connection-error recovery: re-check the daemon, then re-run the last prompt.
            vscode.postMessage({ type: 'checkConnection' });
            if (currentPrompt) sendIntentWithPrompt(currentPrompt);
        }

        function updateGroupedGates() {
            const count = accumulatedGates.length;
            const title = 'Verification Gates (' + count + ' ' + (count === 1 ? 'check' : 'checks') + ')';
            
            let allPassed = true;
            let cardsHtml = '';
            
            for (const gate of accumulatedGates) {
                if (!gate.passed) allPassed = false;

                // Severity comes from the daemon's structured {level}; fall back to passed.
                const level = gate.level || (gate.passed ? 'none' : 'error');
                let sev, icon, iconColor;
                if (level === 'error') {
                    sev = 'failed'; icon = '✗'; iconColor = 'var(--vscode-errorForeground, #ef4444)';
                } else if (level === 'warn') {
                    sev = 'warn'; icon = '⚠'; iconColor = 'var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d)';
                } else {
                    sev = 'passed'; icon = '✓'; iconColor = 'var(--vscode-gitDecoration-addedResourceForeground, #10b981)';
                }
                const badgeText = level === 'none' ? 'passed' : level;

                const reasons = Array.isArray(gate.reasons) ? gate.reasons.filter(r => r && String(r).trim()) : [];
                const reasonsHtml = reasons.length
                    ? '<ul class="gate-reasons">' + reasons.map(r => '<li>' + escapeHtml(r) + '</li>').join('') + '</ul>'
                    : '';
                // Only show the raw details blob when it adds something beyond reasons[].
                const showDetails = gate.details && gate.details.trim() && gate.details.trim() !== reasons.join('\\n').trim();

                cardsHtml +=
                    '<div class="gate-verdict-card ' + sev + '" style="margin-bottom: 4px;">' +
                        '<div style="display: flex; align-items: center; gap: 6px;">' +
                            '<span style="color: ' + iconColor + ';">' + icon + '</span>' +
                            '<strong>' + escapeHtml(gate.gateName) + '</strong>' +
                            '<span style="color: var(--text-muted);">(' + escapeHtml(gate.rule) + ')</span>' +
                        '</div>' +
                        '<span class="gate-badge ' + sev + '">' + escapeHtml(badgeText) + '</span>' +
                    '</div>' +
                    reasonsHtml +
                    (showDetails ?
                        '<details style="margin-top: 2px; margin-bottom: 6px;">' +
                            '<summary style="cursor: pointer; font-size: 10px; color: var(--text-muted); outline: none;">Diagnostic details</summary>' +
                            '<pre style="margin-top: 4px; background: var(--console-bg); border: 1px solid var(--console-border); border-radius: 4px; padding: 6px; font-size: 10px; overflow-x: auto; color: var(--console-fg);"><code style="background:transparent; padding:0;">' + gate.details.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;") + '</code></pre>' +
                        '</details>' : '');
            }
            
            if (!activeGroupedGateItem) {
                activeGroupedGateItem = addTimelineItem(allPassed ? 'success' : 'error', title, cardsHtml);
            } else {
                const titleEl = activeGroupedGateItem.querySelector('.timeline-title-text');
                if (titleEl) titleEl.innerText = title;
                
                const badge = activeGroupedGateItem.querySelector('.timeline-badge');
                if (badge) {
                    badge.className = 'timeline-badge ' + (allPassed ? 'success' : 'error');
                    badge.innerText = allPassed ? '✓' : '✗';
                }
                
                const detailsEl = activeGroupedGateItem.querySelector('.timeline-details');
                if (detailsEl) detailsEl.innerHTML = cardsHtml;
            }
        }

        function autoExportTrajectory() {
            if (localStorage.getItem('monotonicityFilter') === 'true' && currentOutcome !== 'success') {
                console.log('Skipping auto trajectory export due to monotonicity filter (outcome is not success).');
                return;
            }
            const trajectory = {
                sessionId: currentSessionId || 'default',
                prompt: currentPrompt,
                filesRead: currentFilesRead,
                proposals: currentProposals,
                gates: currentGates,
                response: currentResponse,
                outcome: currentOutcome,
                timestamp: new Date().toISOString()
            };
            vscode.postMessage({
                type: 'exportTrajectory',
                trajectory,
                sessionId: currentSessionId || 'default',
                isAuto: true
            });
        }

        function exportTrajectory() {
            const trajectory = {
                sessionId: currentSessionId || 'default',
                prompt: currentPrompt,
                filesRead: currentFilesRead,
                proposals: currentProposals,
                gates: currentGates,
                response: currentResponse,
                outcome: currentOutcome,
                timestamp: new Date().toISOString()
            };
            vscode.postMessage({
                type: 'exportTrajectory',
                trajectory,
                sessionId: currentSessionId || 'default',
                isAuto: false
            });
        }

        // Receive message from extension backend
        window.addEventListener('message', event => {
            const message = event.data;
            switch (message.type) {
                case 'config': {
                    const config = message.config || {};
                    const endpoint = config.llm_endpoint || 'http://127.0.0.1:1234/v1/chat/completions';
                    const model = config.llm_model || 'gemma-4-e2b-it-mlx';
                    const excluded = config.excluded_files ? config.excluded_files.join(', ') : '';
                    const monotonicity = config.monotonicity_filter !== undefined ? config.monotonicity_filter : false;
                    
                    localStorage.setItem('llmEndpoint', endpoint);
                    localStorage.setItem('llmModel', model);
                    localStorage.setItem('excludedFiles', excluded);
                    localStorage.setItem('monotonicityFilter', monotonicity ? 'true' : 'false');
                    
                    const elEnd = document.getElementById('settingLlmEndpoint');
                    const elMod = document.getElementById('settingLlmModel');
                    const elExc = document.getElementById('settingExcludedFiles');
                    const elMono = document.getElementById('settingMonotonicity');
                    if (elEnd) elEnd.value = endpoint;
                    if (elMod) elMod.value = model;
                    if (elExc) elExc.value = excluded;
                    if (elMono) elMono.checked = monotonicity;
                    
                    updateActiveScopeBanner(config.excluded_files || []);
                    break;
                }
                case 'connectionStatus':
                    const badge = document.getElementById('statusBadge');
                    const stext = document.getElementById('statusText');
                    const greet = document.getElementById('greetingBubble');
                    if (badge) {
                        if (message.connected) {
                            badge.className = 'status-badge connected';
                            badge.title = 'FreeCode daemon: online (127.0.0.1:50051)';
                            if (stext) stext.innerText = 'Online';
                        } else {
                            badge.className = 'status-badge offline';
                            badge.title = 'FreeCode daemon: offline — start it';
                            if (stext) stext.innerText = 'Offline';
                        }
                    }
                    // Keep the greeting honest about whether the daemon is actually reachable.
                    if (greet) {
                        greet.innerHTML = message.connected
                            ? 'FreeCode is ready 🏴‍☠️'
                            : '⚠ FreeCode daemon offline — start it, then say hi.';
                    }
                    break;
                case 'files_read': {
                    try {
                        // Dedupe exact paths; disambiguate colliding basenames with their parent dir
                        // (so daemon/main.rs and cli/main.rs aren't two identical "main.rs" chips).
                        const files = Array.from(new Set(JSON.parse(message.message)));
                        const baseCounts = {};
                        files.forEach(f => { const b = f.split('/').pop(); baseCounts[b] = (baseCounts[b] || 0) + 1; });
                        if (files && files.length > 0) {
                            const tagsHtml = files.map(f => {
                                const parts = f.split('/');
                                const basename = parts.pop();
                                const label = (baseCounts[basename] > 1 && parts.length) ? parts.pop() + '/' + basename : basename;
                                return '<span class="file-read-tag" data-action="openFile" data-a1="' + escapeHtml(f) + '" title="' + escapeHtml(f) + '"><span class="file-read-icon">📄</span>' + escapeHtml(label) + '</span>';
                            }).join('');
                            
                            const detailsHtml = 
                                '<details open>' +
                                    '<summary style="cursor: pointer; font-size: 10px; color: var(--text-muted); outline: none;">Context Files Scanned (' + files.length + ')</summary>' +
                                    '<div class="files-read-container">' + tagsHtml + '</div>' +
                                '</details>';
                            addTimelineItem('success', 'Assembled context from workspace', detailsHtml);
                            if (fcState) { fcState.ctx = files.length; if (fcState.phase === 'intent') fcState.phase = 'ctx'; renderFcStrip(); }
                        }
                    } catch (err) {
                        console.error('Error rendering files_read event:', err);
                    }
                    break;
                }
                case 'proposal': {
                    // The one moving part of the scope bar: this location is being written NOW.
                    markScopeWritten();
                    try {
                        const data = JSON.parse(message.message);
                        currentProposals.push({
                            filePath: data.filePath,
                            oldContent: data.oldContent,
                            newContent: data.newContent
                        });
                        accumulatedProposals.push(data);
                        updateGroupedProposals();
                    } catch (err) {
                        console.error('Error rendering proposal event:', err);
                    }
                    break;
                }
                case 'gate_verdict': {
                    try {
                        const data = JSON.parse(message.message);
                        currentGates.push({
                            gateName: data.gateName,
                            rule: data.rule,
                            passed: data.passed,
                            level: data.level,
                            reasons: data.reasons,
                            details: data.details
                        });
                        accumulatedGates.push(data);
                        updateGroupedGates();
                        fcSyncGates(); renderFcStrip();
                    } catch (err) {
                        console.error('Error rendering gate_verdict event:', err);
                    }
                    break;
                }
                case 'tool_call': {
                    try {
                        const d = JSON.parse(message.message);
                        let preview = '';
                        try {
                            const a = JSON.parse(d.arguments);
                            preview = Object.entries(a).map(([k, v]) => {
                                const s = typeof v === 'string' ? v : JSON.stringify(v);
                                return k + ': ' + (s.length > 60 ? s.slice(0, 60) + '…' : s);
                            }).join('  ·  ');
                        } catch (e) { preview = String(d.arguments || '').slice(0, 80); }
                        const details = '<div style="font-size: 10px; color: var(--text-muted); white-space: pre-wrap; word-break: break-word;">' + escapeHtml(preview) + '</div>';
                        const el = addTimelineItem('running', '🔧 ' + (d.name || 'tool'), details);
                        if (d.id) { toolCallEls[d.id] = el; }
                    } catch (err) { console.error('Error rendering tool_call event:', err); }
                    break;
                }
                case 'tool_result': {
                    try {
                        const d = JSON.parse(message.message);
                        const result = String(d.result || '');
                        const lower = result.toLowerCase();
                        let status = 'success', icon = '✓';
                        if (result.startsWith('error') || lower.includes('failed')) { status = 'error'; icon = '✗'; }
                        else if (result.startsWith('note')) { status = 'running'; icon = '•'; }
                        const shown = result.length > 400 ? result.slice(0, 400) + '…' : result;
                        const resBlock = '<div style="margin-top: 4px; font-size: 10px; white-space: pre-wrap; word-break: break-word; color: var(--text-muted);">' + escapeHtml(shown) + '</div>';
                        const el = d.tool_call_id ? toolCallEls[d.tool_call_id] : null;
                        if (el) {
                            const badge = el.querySelector('.timeline-badge');
                            if (badge) { badge.className = 'timeline-badge ' + status; badge.innerText = icon; }
                            const det = el.querySelector('.timeline-details');
                            if (det) { det.innerHTML += resBlock; }
                            else {
                                const content = el.querySelector('.timeline-content');
                                if (content) { content.innerHTML += '<div class="timeline-details">' + resBlock + '</div>'; }
                            }
                            if (d.tool_call_id) { delete toolCallEls[d.tool_call_id]; }
                        } else {
                            addTimelineItem(status, '🔧 ' + (d.name || 'tool') + ' result', resBlock);
                        }
                    } catch (err) { console.error('Error rendering tool_result event:', err); }
                    break;
                }
                case 'hitlPending': {
                    addTimelineItem('running', 'Human gate approval required: check workspace diffs or use action buttons above');
                    break;
                }
                case 'hitlDecisionApplied': {
                    const decision = message.decision;
                    pendingProposals.forEach(prop => {
                        const cacheKey = prop.filePath + ":" + prop.contentHash;
                        decisionCache[cacheKey] = decision;
                        
                        const badge = document.getElementById('badge-' + prop.id);
                        if (badge) {
                            badge.className = 'proposal-status-badge ' + (decision === 'accept' ? 'applied' : 'discarded');
                            badge.innerText = decision === 'accept' ? 'Applied' : 'Discarded';
                        }
                        
                        const container = document.getElementById('actions-' + prop.id);
                        if (container) {
                            container.innerHTML = 
                                '<span class="proposal-status-badge ' + (decision === 'accept' ? 'applied' : 'discarded') + '">' +
                                    (decision === 'accept' ? 'Changes Applied' : 'Changes Discarded') +
                                '</span>';
                        }
                    });
                    pendingProposals = [];
                    break;
                }
                case 'step':
                    resolveActiveStep();
                    if (fcState) {
                        const fm = (message.message || '').match(/turn (\\d+)\\/(\\d+)/);
                        if (fm) { fcState.at = +fm[1]; fcState.am = +fm[2]; fcState.phase = 'agent'; renderFcStrip(); }
                    }

                    let formattedText = message.message;
                    if (message.message.startsWith('✓ Wrote file to: ') || message.message.includes('✓ Wrote file')) {
                        addTimelineItem('success', formattedText);
                    } else if (message.message.startsWith('✗ Error writing to') || message.message.includes('✗ Error writing')) {
                        addTimelineItem('error', formattedText);
                    } else {
                        const stepEl = addTimelineItem(message.status === 'info' ? 'running' : message.status, formattedText);
                        if (message.status === 'info') {
                            activeStep = stepEl;
                        }
                    }
                    
                    // Reset AST spinner on finish
                    if (message.message.includes('AST Edit Applied') || message.message.includes('AST Edit Refused') || message.message.includes('AST Error')) {
                        document.getElementById('astSpinner').style.display = 'none';
                        document.getElementById('applyAstBtn').disabled = false;
                        refreshGitStatus();
                    }
                    break;
                case 'token':
                    resolveActiveStep();
                    if (!activeAssistantMessageBubble) {
                        createAssistantPlaceholder();
                    }
                    activeAssistantMessageText += message.message;
                    activeAssistantMessageBubble.innerHTML = parseMarkdown(activeAssistantMessageText);
                    const chatContainer = document.getElementById('chatContainer');
                    chatContainer.scrollTop = chatContainer.scrollHeight;
                    break;
                case 'response':
                    resolveActiveStep();

                    document.getElementById('sendBtn').style.display = 'flex';
                    document.getElementById('sendBtn').disabled = false;
                    document.getElementById('stopBtn').style.display = 'none';
                    
                    let targetBubble = null;
                    if (activeAssistantMessageBubble) {
                        activeAssistantMessageBubble.innerHTML = parseMarkdown(message.message);
                        targetBubble = activeAssistantMessageBubble;
                        activeAssistantMessageBubble = null;
                        activeAssistantMessageText = "";
                    } else {
                        appendMessage('assistant', message.message);
                        const bubbles = document.querySelectorAll('.message.assistant .message-bubble');
                        if (bubbles.length > 0) {
                            targetBubble = bubbles[bubbles.length - 1];
                        }
                    }
                    
                    refreshGitStatus();
                    
                    if (activeTimer) {
                        clearInterval(activeTimer);
                        activeTimer = null;
                    }
                    isRunActive = false;
                    
                    // PIC-10: settle into the one-line receipt, then move it BELOW this turn's answer
                    // bubble (decision #3 — strip above while running, receipt below when settled).
                    if (fcState) { fcState.lat = runLatency; fcState.done = true; fcState.phase = 'done'; renderFcStrip(); }
                    if (activeTimeline) {
                        const cc2 = document.getElementById('chatContainer');
                        if (cc2) { cc2.appendChild(activeTimeline); cc2.scrollTop = cc2.scrollHeight; }
                    }
                    // Reset active timeline for next request
                    activeTimeline = null;
                    fcStripEl = null;

                    currentResponse = message.message;
                    currentOutcome = message.status || 'success';
                    currentSessionId = message.sessionId || 'default';
                    
                    const isConnError = message.errorKind === 'offline' || message.errorKind === 'daemon';
                    const isFailure = message.status === 'error' ||
                                      message.message.includes('Compilation error') ||
                                      message.message.includes('Attempts exhausted') ||
                                      message.message.includes('✗ **Compilation error');

                    if (isFailure && targetBubble) {
                        // Make an error read as an error, not a chummy FreeCode reply.
                        const msgEl = targetBubble.closest('.message');
                        if (msgEl) {
                            msgEl.classList.add('error');
                            const senderEl = msgEl.querySelector('.message-sender');
                            if (senderEl) senderEl.innerText = isConnError ? 'Connection error' : 'FreeCode — error';
                        }
                        const recoveryContainer = document.createElement('div');
                        recoveryContainer.className = 'recovery-retry-container';
                        if (isConnError) {
                            // A correction hint can't fix a connection error — offer reconnect + retry instead.
                            recoveryContainer.innerHTML =
                                '<button class="recovery-btn" data-action="retryLastPrompt">Reconnect &amp; retry</button>';
                        } else {
                            const recoveryId = 'rec_' + Date.now();
                            recoveryContainer.innerHTML =
                                '<input type="text" class="recovery-input" id="input-' + recoveryId + '" placeholder="Provide correction hint (e.g., fix type/import)..." />' +
                                '<button class="recovery-btn" id="btn-' + recoveryId + '" data-action="submitRecoveryRetry" data-a1="' + escapeHtml(recoveryId) + '">Retry with Correction</button>';
                        }
                        targetBubble.appendChild(recoveryContainer);
                    }

                    autoExportTrajectory();
                    break;
                case 'metrics':
                    onDaemonMetricsReceived(message.metrics);
                    break;
                case 'gitStatus':
                    updateGitPanel(message.status);
                    break;
                case 'scope':
                    scopeHome = (message.scope && message.scope.home) || '';
                    scopeEntries = (message.scope && message.scope.entries) || [];
                    renderScope();
                    break;
                case 'memories':
                    updateMemoryLists(message.project, message.global);
                    break;
                case 'activeFilePath':
                    if (message.filePath) {
                        document.getElementById('filePathInput').value = message.filePath;
                    }
                    break;
            }
        });

        // Harness Observability Functions
        function toggleParamsSection() {
            const content = document.getElementById('paramsContent');
            isParamsOpen = !isParamsOpen;
            if (isParamsOpen) {
                content.style.display = 'flex';
            } else {
                content.style.display = 'none';
            }
        }

        function updateRangeLabel(name) {
            const range = document.getElementById('param' + name);
            const label = document.getElementById('val' + name);
            if (range && label) {
                if (name === 'G') {
                    label.innerText = Math.round(parseFloat(range.value) * 100) + '%';
                } else {
                    label.innerText = parseFloat(range.value).toFixed(2);
                }
            }
        }

        function stylePayoff(elementId, value) {
            const el = document.getElementById(elementId);
            if (!el) return;
            if (value > 0) {
                el.className = 'harness-val positive';
            } else if (value < 0) {
                el.className = 'harness-val negative';
            } else {
                el.className = 'harness-val';
            }
        }

        function calculateHarnessMath() {
            // Reset to theoretical expected view if parameters are modified
            isRunActive = false;
            if (activeTimer) {
                clearInterval(activeTimer);
                activeTimer = null;
            }

            const badge = document.getElementById('harnessModeBadge');
            if (badge) {
                badge.innerText = 'Expected';
                badge.style.background = 'var(--btn-secondary-bg)';
                badge.style.color = 'var(--text-muted)';
                badge.style.borderColor = 'var(--card-border)';
            }

            // Restore expected tooltips
            document.getElementById('valPayoff').parentElement.title = 'Expected net payoff: s * V - e * L - Cost';
            document.getElementById('valCost').parentElement.title = 'Expected total cost: E[N] * (C_gen + C_ver) + κ * E[latency]';

            const V = parseFloat(document.getElementById('paramV').value) || 0;
            const L = parseFloat(document.getElementById('paramL').value) || 0;
            const kappa = parseFloat(document.getElementById('paramKappa').value) || 0;
            const C_gen = parseFloat(document.getElementById('paramCgen').value) || 0;
            const C_ver = parseFloat(document.getElementById('paramCver').value) || 0;
            const alpha = parseFloat(document.getElementById('paramAlpha').value) || 0;
            const beta = parseFloat(document.getElementById('paramBeta').value) || 0;
            const R = parseInt(document.getElementById('paramR').value) || 0;
            const g = parseFloat(document.getElementById('paramG').value) || 0.80;

            currentConfidence = g;

            const p = g * (1 - alpha) + (1 - g) * beta;
            const totalMaxRuns = R + 1;

            let E_N = 0;
            if (p > 0) {
                E_N = (1 - Math.pow(1 - p, totalMaxRuns)) / p;
            } else {
                E_N = totalMaxRuns;
            }

            let s = 0;
            if (p > 0) {
                s = (g * (1 - alpha) / p) * (1 - Math.pow(1 - p, totalMaxRuns));
            }

            let e = 0;
            if (p > 0) {
                e = ((1 - g) * beta / p) * (1 - Math.pow(1 - p, totalMaxRuns));
            }

            const estimated_latency = E_N * 8.0;
            const expected_cost = E_N * (C_gen + C_ver) + kappa * estimated_latency;
            const expected_payoff = s * V - e * L - expected_cost;

            document.getElementById('valPayoff').innerText = '€' + expected_payoff.toFixed(2);
            stylePayoff('valPayoff', expected_payoff);
            
            document.getElementById('valCost').innerText = '€' + expected_cost.toFixed(2);
            document.getElementById('valConfidence').innerText = (g * 100).toFixed(1) + '%';
            document.getElementById('valAttempts').innerText = E_N.toFixed(2) + ' / ' + totalMaxRuns;
        }

        function onDaemonMetricsReceived(metrics) {
            if (activeTimer) {
                clearInterval(activeTimer);
                activeTimer = null;
            }
            isRunActive = false;

            const runLatency = metrics.total_latency;
            const actualAttemptsCount = metrics.attempt_count;
            const compLatency = metrics.compilation_latency;
            const modelLatency = metrics.model_latency;

            const V = parseFloat(document.getElementById('paramV').value) || 0;
            const L = parseFloat(document.getElementById('paramL').value) || 0;
            const kappa = parseFloat(document.getElementById('paramKappa').value) || 0;
            const C_gen = parseFloat(document.getElementById('paramCgen').value) || 0;
            const C_ver = parseFloat(document.getElementById('paramCver').value) || 0;

            const actual_cost = actualAttemptsCount * (C_gen + C_ver) + kappa * runLatency;

            let actual_payoff = 0;
            let outcomeText = '';
            
            if (metrics.success) {
                actual_payoff = V - actual_cost;
                outcomeText = 'Resolved (s=1, e=0)';
            } else {
                actual_payoff = -actual_cost;
                outcomeText = 'Error (s=0, e=0)';
            }

            document.getElementById('valPayoff').innerText = '€' + actual_payoff.toFixed(2);
            stylePayoff('valPayoff', actual_payoff);
            
            document.getElementById('valCost').innerText = '€' + actual_cost.toFixed(2);
            document.getElementById('valConfidence').innerText = (currentConfidence * 100).toFixed(1) + '%';
            document.getElementById('valAttempts').innerText = actualAttemptsCount + ' (' + runLatency.toFixed(1) + 's)';
            
            // Set badge to actual (green style)
            const badge = document.getElementById('harnessModeBadge');
            if (badge) {
                badge.innerText = 'Actual';
                badge.style.background = 'color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 15%, transparent)';
                badge.style.color = 'var(--vscode-gitDecoration-addedResourceForeground, #10b981)';
                badge.style.borderColor = 'color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 30%, transparent)';
            }

            // Set tooltips with detailed latency breakdown
            document.getElementById('valPayoff').parentElement.title = 'Actual run payoff: ' + outcomeText + '. Formula: s*V - e*L - Cost';
            document.getElementById('valCost').parentElement.title = 'Actual run cost: ' + actualAttemptsCount + ' attempts + latency of ' + runLatency.toFixed(1) + 's (LLM: ' + modelLatency.toFixed(1) + 's, Compiler: ' + compLatency.toFixed(1) + 's)';

            // Accumulate cumulative metrics
            const promptTokens = metrics.prompt_tokens || 0;
            const completionTokens = metrics.completion_tokens || 0;
            const llmCalls = metrics.llm_calls || actualAttemptsCount;

            sessionCalls += llmCalls;
            sessionTokens += Math.round(promptTokens + completionTokens);
            sessionLatency += modelLatency;
            sessionSpent += actual_cost;

            // Update cumulative UI
            const elTokens = document.getElementById('valSessionTokens');
            const elLatency = document.getElementById('valSessionLatency');
            const elCalls = document.getElementById('valSessionCalls');
            const elSpent = document.getElementById('valSessionSpent');
            if (elTokens) elTokens.innerText = sessionTokens;
            if (elLatency) elLatency.innerText = sessionLatency.toFixed(1) + 's';
            if (elCalls) elCalls.innerText = sessionCalls;
            if (elSpent) elSpent.innerText = '€' + sessionSpent.toFixed(2);
        }

        // Initialize Harness panel on load
        setTimeout(calculateHarnessMath, 200);

        // ------------------------------------------------------------------
        // Event delegation.
        //
        // Every interactive element declares what it does with data-action (click), or
        // data-change / data-input / data-mousedown / data-keydown for the other events, plus
        // data-a1 / data-a2 for arguments and data-stop="1" to stopPropagation first.
        //
        // Two things this buys, beyond letting the CSP drop 'unsafe-inline':
        //   1. Arguments live in attributes, never in a JS string built by concatenation.
        //   2. Dispatch goes through this explicit table, so markup can only ever reach a
        //      function listed here - not any global that happens to share the name.
        // ------------------------------------------------------------------
        const FC_ACTIONS = {
            toggleGitPanel:      function (el, ev) { toggleGitPanel(); },
            toggleMemoryPanel:   function (el, ev) { toggleMemoryPanel(); },
            toggleHarnessPanel:  function (el, ev) { toggleHarnessPanel(); },
            toggleSettingsPanel: function (el, ev) { toggleSettingsPanel(); },
            toggleAstSection:    function (el, ev) { toggleAstSection(); },
            toggleParamsSection: function (el, ev) { toggleParamsSection(); },
            fcCycleMode:         function (el, ev) { fcCycleMode(); },
            toggleMemorySubSection: function (el, ev) { toggleMemorySubSection(el.dataset.a1); },

            pingDaemon:       function (el, ev) { pingDaemon(); },
            refreshGitStatus: function (el, ev) { refreshGitStatus(); },
            saveSettings:     function (el, ev) { saveSettings(); },
            exportTrajectory: function (el, ev) { exportTrajectory(); },
            applyAstEdit:     function (el, ev) { applyAstEdit(); },
            clearChat:        function (el, ev) { clearChat(); },
            sendIntent:       function (el, ev) { sendIntent(); },
            stopIntent:       function (el, ev) { stopIntent(); },
            retryLastPrompt:  function (el, ev) { retryLastPrompt(); },
            setMode:          function (el, ev) { setMode(el.dataset.a1); },

            openFile: function (el, ev) { openFile(el.dataset.a1); },
            openDiff: function (el, ev) { openDiff(el.dataset.a1); },

            addMemoryNote:    function (el, ev) { addMemoryNote(el.dataset.a1); },
            startEditMemory:  function (el, ev) { startEditMemory(el.dataset.a1, el.dataset.a2); },
            saveEditMemory:   function (el, ev) { saveEditMemory(el.dataset.a1, el.dataset.a2); },
            deleteMemoryNote: function (el, ev) { deleteMemoryNote(el.dataset.a1, el.dataset.a2); },
            cancelEditMemory: function (el, ev) { cancelEditMemory(el.dataset.a1); },

            fcExpand:       function (el, ev) { fcExpand(el); },
            toggleEditView: function (el, ev) { toggleEditView(el, el.dataset.a1, Number(el.dataset.a2)); },
            submitGroupedHitlDecision: function (el, ev) { submitGroupedHitlDecision(el.dataset.a1, el.dataset.a2); },
            submitRecoveryRetry:       function (el, ev) { submitRecoveryRetry(el.dataset.a1); },

            initResize:           function (el, ev) { initResize(ev, el.dataset.a1); },
            handleKeyPress:       function (el, ev) { handleKeyPress(ev); },
            calculateHarnessMath: function (el, ev) { calculateHarnessMath(); },
            rangeInput:           function (el, ev) { updateRangeLabel(el.dataset.a1); calculateHarnessMath(); },
            updateProposedCode:   function (el, ev) { updateProposedCode(el.dataset.a1, Number(el.dataset.a2)); },
            toggleScope:          function (el, ev) { toggleScope(); },
            toggleModeMenu:       function (el, ev) { ev.stopPropagation(); toggleModeMenu(); }
        };

        function fcDispatch(ev, attr) {
            const target = ev.target;
            if (!target || typeof target.closest !== 'function') return;
            const el = target.closest('[data-' + attr + ']');
            if (!el) return;
            const name = el.getAttribute('data-' + attr);
            const fn = FC_ACTIONS[name];
            if (typeof fn !== 'function') {
                console.warn('FreeCode: unknown data-' + attr + ' action: ' + name);
                return;
            }
            if (el.dataset.stop === '1') ev.stopPropagation();
            fn(el, ev);
        }

        document.addEventListener('click', function (ev) {
            // Close the mode menu on any click that is not on the trigger or inside the menu.
            const t = ev.target;
            if (t && typeof t.closest === 'function'
                && !t.closest('#modeMenu') && !t.closest('#modeTrigger')) {
                closeModeMenu();
            }
            fcDispatch(ev, 'action');
        });
        document.addEventListener('change',    function (ev) { fcDispatch(ev, 'change'); });
        document.addEventListener('input',     function (ev) { fcDispatch(ev, 'input'); });
        document.addEventListener('mousedown', function (ev) { fcDispatch(ev, 'mousedown'); });
        document.addEventListener('keydown',   function (ev) { fcDispatch(ev, 'keydown'); });

        // Load config from workspace config.json
        vscode.postMessage({ type: 'readConfig' });
        vscode.postMessage({ type: 'getScope' });
        setMode(currentMode);
    `;
}
