export function getWebviewCss(): string {
    // Air-gap: NO remote @import. FreeCode is local-first — the webview must render
    // identically with zero network. Fonts resolve from VS Code's own configured
    // families first, then the OS stack; nothing is ever fetched.
    return `:root {
        /* 100% Decoupled Theme System using VS Code Native Theme Tokens */
        --bg-color: var(--vscode-sideBar-background, var(--vscode-editor-background, #1e1e1e));
        --text-main: var(--vscode-sideBar-foreground, var(--vscode-foreground, #cccccc));
        --text-muted: var(--vscode-descriptionForeground, #8c8c8c);
        
        --card-bg: var(--vscode-welcomePage-tile-background, var(--vscode-editorWidget-background, rgba(128, 128, 128, 0.08)));
        --card-border: var(--vscode-widget-border, var(--vscode-editorWidget-border, rgba(128, 128, 128, 0.2)));
        
        --input-bg: var(--vscode-input-background, #252526);
        --input-fg: var(--vscode-input-foreground, #cccccc);
        --input-border: var(--vscode-input-border, rgba(128, 128, 128, 0.25));
        --input-placeholder: var(--vscode-input-placeholderForeground, #8c8c8c);
        --focus-border: var(--vscode-focusBorder, #007acc);

        --btn-primary-bg: var(--vscode-button-background, #007acc);
        --btn-primary-hover: var(--vscode-button-hoverBackground, #0062a3);
        --btn-primary-fg: var(--vscode-button-foreground, #ffffff);

        --btn-secondary-bg: var(--vscode-button-secondaryBackground, rgba(128, 128, 128, 0.1));
        --btn-secondary-hover: var(--vscode-button-secondaryHoverBackground, rgba(128, 128, 128, 0.15));
        --btn-secondary-fg: var(--vscode-button-secondaryForeground, #ffffff);

        --console-bg: var(--vscode-terminal-background, #1e1e1e);
        --console-fg: var(--vscode-terminal-foreground, #cccccc);
        --console-border: var(--vscode-panel-border, rgba(128, 128, 128, 0.2));

        --font-family: var(--vscode-font-family, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif);
        --mono-font: var(--vscode-editor-font-family, "Courier New", monospace);
    }

    * {
        box-sizing: border-box;
        margin: 0;
        padding: 0;
    }

    body {
        background-color: var(--bg-color);
        color: var(--text-main);
        font-family: var(--font-family);
        font-size: 13px;
        line-height: 1.5;
        padding: 0;
        margin: 0;
        display: flex;
        flex-direction: column;
        height: 100vh;
        overflow: hidden;
    }

    /* Custom Scrollbars matching VS Code Slider styling */
    ::-webkit-scrollbar {
        width: 10px;
        height: 10px;
    }

    ::-webkit-scrollbar-track {
        background: transparent;
    }

    ::-webkit-scrollbar-thumb {
        background: var(--vscode-scrollbarSlider-background, rgba(121, 121, 121, 0.4));
        border-radius: 0px;
    }

    ::-webkit-scrollbar-thumb:hover {
        background: var(--vscode-scrollbarSlider-hoverBackground, rgba(100, 100, 100, 0.7));
    }

    ::-webkit-scrollbar-thumb:active {
        background: var(--vscode-scrollbarSlider-activeBackground, rgba(0, 0, 0, 0.6));
    }

    /* Selection styling matching VS Code editor selection */
    ::selection {
        background: var(--vscode-editor-selectionBackground, rgba(0, 122, 204, 0.3));
        color: inherit;
    }

    /* Keyboard accessibility focus styling */
    button:focus-visible, 
    a:focus-visible,
    textarea:focus-visible,
    input:focus-visible,
    .mode-trigger:focus-visible,
    .mode-option:focus-visible,
    .scope-summary:focus-visible,
    .status-badge:focus-visible,
    .header-btn:focus-visible {
        outline: 1px solid var(--focus-border);
        outline-offset: 2px;
    }

    /* General Link Styling */
    a {
        color: var(--vscode-textLink-foreground, #3794ff);
        text-decoration: none;
        cursor: pointer;
    }
    a:hover {
        color: var(--vscode-textLink-activeForeground, #3794ff);
        text-decoration: underline;
    }

    /* Top Header - Frosted Glass Sticky */
    .header {
        position: sticky;
        top: 0;
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 10px 14px;
        border-bottom: 1px solid var(--card-border);
        background: color-mix(in srgb, var(--bg-color) 85%, transparent);
        backdrop-filter: blur(8px);
        -webkit-backdrop-filter: blur(8px);
        z-index: 100;
    }

    .logo {
        font-size: 13px;
        font-weight: 700;
        background: linear-gradient(135deg, var(--btn-primary-bg) 0%, var(--focus-border) 100%);
        -webkit-background-clip: text;
        -webkit-text-fill-color: transparent;
        letter-spacing: 1.5px;
    }

    .header-actions {
        display: flex;
        align-items: center;
        gap: 8px;
    }

/* .active-scope-banner removed: the write-scope bar absorbed it (net decorations: zero). */

    /* Header buttons */
    .header-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 20px;
        height: 20px;
        padding: 0;
        border-radius: 4px;
        background: var(--btn-secondary-bg);
        color: var(--text-main);
        border: 1px solid var(--card-border);
        cursor: pointer;
        transition: all 0.2s ease;
    }
    .header-btn:hover {
        background: var(--btn-secondary-hover);
        border-color: var(--focus-border);
    }
    .header-btn:active {
        background: color-mix(in srgb, var(--btn-secondary-bg) 80%, black);
    }
    .header-btn.active {
        background: var(--btn-primary-bg);
        color: var(--btn-primary-fg);
        border-color: var(--btn-primary-bg);
    }

    .status-badge {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 5px;
        height: 20px;
        padding: 0 7px;
        border-radius: 4px;
        cursor: pointer;
        transition: all 0.2s ease;
        border: 1px solid var(--card-border);
    }

    .status-text {
        font-size: 10px;
        font-weight: 600;
        letter-spacing: 0.02em;
    }

    .status-badge.offline {
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 15%, transparent);
        color: var(--vscode-errorForeground, #ef4444);
        border-color: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 30%, transparent);
    }

    .status-badge.connected {
        background: color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 15%, transparent);
        color: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
        border-color: color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 30%, transparent);
    }

    .status-badge.checking {
        background: color-mix(in srgb, var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d) 15%, transparent);
        color: var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d);
        border-color: color-mix(in srgb, var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d) 30%, transparent);
    }

    .status-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background-color: currentColor;
        box-shadow: 0 0 4px currentColor;
    }

    .header-icon {
        width: 12px;
        height: 12px;
        stroke: currentColor;
    }

    /* Collapsible Git Status Panel - Frosted Glass Sticky */
    .git-panel {
        position: sticky;
        top: 36px;
        background: color-mix(in srgb, var(--card-bg) 90%, transparent);
        backdrop-filter: blur(8px);
        -webkit-backdrop-filter: blur(8px);
        border-bottom: 1px solid var(--card-border);
        font-size: 11px;
        z-index: 90;
    }
    .git-panel-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        padding: 8px 14px;
        cursor: pointer;
        user-select: none;
        font-weight: 600;
        color: var(--text-muted);
        transition: color 0.2s ease;
    }
    .git-panel-header:hover {
        color: var(--text-main);
    }
    .git-refresh-btn {
        cursor: pointer;
        margin-left: 6px;
        color: var(--text-muted);
        font-size: 12px;
        transition: color 0.2s ease;
    }
    .git-refresh-btn:hover {
        color: var(--text-main);
    }
    .git-file-list {
        display: flex;
        padding: 4px 14px 10px 14px;
        flex-direction: column;
        gap: 4px;
        max-height: 150px;
        overflow-y: auto;
    }
    .git-file-item {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 2px 0;
    }
    .git-file-status {
        font-weight: 700;
        font-size: 9px;
        text-transform: uppercase;
        padding: 1px 4px;
        border-radius: 3px;
        width: 15px;
        text-align: center;
    }
    .git-file-status.modified {
        background: color-mix(in srgb, var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d) 15%, transparent);
        color: var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d);
    }
    .git-file-status.added {
        background: color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 15%, transparent);
        color: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
    }
    .git-file-status.deleted {
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 15%, transparent);
        color: var(--vscode-errorForeground, #ef4444);
    }
    .git-file-name {
        color: var(--vscode-textLink-foreground, #3794ff);
        text-decoration: none;
        cursor: pointer;
    }
    .git-file-name:hover {
        text-decoration: underline;
    }
    .git-file-actions {
        display: flex;
        align-items: center;
        gap: 4px;
        margin-left: auto;
    }
    .git-diff-btn {
        font-size: 10px;
        color: var(--text-muted);
        cursor: pointer;
        padding: 1px 4px;
        border-radius: 3px;
        transition: all 0.2s ease;
    }
    .git-diff-btn:hover {
        color: var(--vscode-textLink-foreground, #3794ff);
        background: rgba(128, 128, 128, 0.1);
    }

    .panel-resizer {
        height: 6px;
        margin: 8px -14px -12px -14px;
        cursor: ns-resize;
        background: transparent;
        border-top: 1px solid var(--card-border);
        position: relative;
        z-index: 10;
    }
    .panel-resizer:hover {
        background: color-mix(in srgb, var(--focus-border) 30%, transparent);
    }

    /* AST Drawer */
    .ast-card {
        display: none;
        background: var(--card-bg);
        border-bottom: 1px solid var(--card-border);
        padding: 12px 14px;
        animation: slideDown 0.2s ease-out;
    }

    .ast-card.open {
        display: block;
    }

    @keyframes slideDown {
        from { transform: translateY(-8px); opacity: 0; }
        to { transform: translateY(0); opacity: 1; }
    }

    .ast-card-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-bottom: 8px;
    }

    .ast-card-header h4 {
        margin: 0;
        font-size: 10px;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-muted);
    }

    /* Input Placeholders styling */
    .input-text::placeholder,
    .textarea::placeholder,
    .chat-input::placeholder {
        color: var(--input-placeholder);
        opacity: 1;
    }

    .input-text {
        width: 100%;
        background: var(--input-bg);
        border: 1px solid var(--input-border);
        border-radius: 6px;
        color: var(--input-fg);
        padding: 6px 8px;
        font-family: var(--font-family);
        font-size: 11px;
        outline: none;
        margin-bottom: 6px;
    }

    .input-text:focus {
        border-color: var(--focus-border);
    }

    .textarea {
        width: 100%;
        background: var(--input-bg);
        border: 1px solid var(--input-border);
        border-radius: 6px;
        color: var(--input-fg);
        padding: 6px 8px;
        font-family: var(--font-family);
        font-size: 11px;
        outline: none;
        resize: vertical;
        margin-bottom: 8px;
    }

    .textarea:focus {
        border-color: var(--focus-border);
    }

    .btn-secondary {
        background: var(--btn-secondary-bg);
        color: var(--btn-secondary-fg);
        border: 1px solid var(--input-border);
        width: 100%;
        padding: 6px;
        border-radius: 6px;
        font-size: 11px;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.2s ease;
    }

    .btn-secondary:hover {
        background: var(--btn-secondary-hover);
    }
    .btn-secondary:active {
        background: color-mix(in srgb, var(--btn-secondary-bg) 80%, black);
    }

    /* Chat Scroll Container */
    .chat-container {
        flex-grow: 1;
        overflow-y: auto;
        padding: 14px;
        display: flex;
        flex-direction: column;
        gap: 14px;
        scroll-behavior: smooth;
    }

    /* Message Bubbles */
    .message {
        display: flex;
        flex-direction: column;
        max-width: 90%;
        animation: fadeIn 0.2s ease-out;
    }

    @keyframes fadeIn {
        from { opacity: 0; transform: translateY(4px); }
        to { opacity: 1; transform: translateY(0); }
    }

    .message.user {
        align-self: flex-end;
        align-items: flex-end;
    }

    .message.assistant {
        align-self: flex-start;
        align-items: flex-start;
        max-width: 100%;
        width: 100%;
    }

    .message-bubble {
        padding: 8px 12px;
        border-radius: 12px;
        font-size: 12.5px;
        line-height: 1.45;
        word-wrap: break-word;
    }

    .message.user .message-bubble {
        background: var(--btn-primary-bg);
        color: var(--btn-primary-fg);
        border-radius: 12px 12px 2px 12px;
        box-shadow: var(--vscode-widget-shadow, 0 1px 4px rgba(0, 0, 0, 0.1));
    }

    .message.assistant .message-bubble {
        background: var(--card-bg);
        color: var(--text-main);
        border: 1px solid var(--card-border);
        border-radius: 12px 12px 12px 2px;
        width: 100%;
    }

    /* Markdown Styles in Messages */
    .message-bubble pre {
        background: var(--console-bg);
        border: 1px solid var(--console-border);
        border-radius: 6px;
        padding: 8px;
        overflow-x: auto;
        margin: 8px 0;
        font-family: var(--mono-font);
        font-size: 11px;
        color: var(--console-fg);
    }

    .message-bubble code {
        font-family: var(--mono-font);
        font-size: 11px;
        background: var(--vscode-textCodeBlock-background, rgba(128, 128, 128, 0.15));
        padding: 2px 4px;
        border-radius: 4px;
    }

    .message-bubble pre code {
        background: transparent;
        padding: 0;
        border-radius: 0;
    }

    /* Clickable file paths */
    code.file-link {
        color: var(--vscode-textLink-foreground, #3794ff);
        text-decoration: underline;
        cursor: pointer;
        transition: color 0.2s ease;
    }
    code.file-link:hover {
        color: var(--vscode-textLink-activeForeground, #3794ff);
    }

    /* Collapsible Compiler Error container */
    .compiler-error-details {
        margin: 10px 0;
        border: 1px solid var(--vscode-errorForeground, #ef4444);
        border-radius: 6px;
        background: var(--console-bg);
        overflow: hidden;
    }
    .compiler-error-details summary {
        padding: 8px 12px;
        font-size: 11px;
        font-weight: 700;
        cursor: pointer;
        color: var(--vscode-errorForeground, #ef4444);
        user-select: none;
        outline: none;
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 10%, transparent);
        border-bottom: 1px solid color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 20%, transparent);
    }
    .compiler-error-details[open] summary {
        border-bottom: 1px solid var(--vscode-errorForeground, #ef4444);
    }
    .compiler-error-details pre {
        margin: 0 !important;
        border: none !important;
        border-radius: 0 !important;
        background: transparent !important;
    }

    .message-bubble h1, .message-bubble h2, .message-bubble h3 {
        font-size: 13px;
        font-weight: 700;
        margin: 10px 0 6px 0;
    }

    .message-bubble ul, .message-bubble ol {
        margin-left: 16px;
        margin-bottom: 8px;
    }

    .message-bubble li {
        margin-bottom: 4px;
    }

    .message-sender {
        font-size: 9px;
        font-weight: 700;
        color: var(--text-muted);
        margin-bottom: 3px;
        text-transform: uppercase;
        letter-spacing: 0.5px;
    }

    /* An error response reads as an error, not a normal FreeCode reply. */
    .message.error .message-sender {
        color: var(--vscode-errorForeground, #ef4444);
    }

    .message.error .message-bubble {
        border: 1px solid color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 35%, transparent);
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 8%, transparent);
    }

    hr {
        border: none;
        border-top: 1px solid var(--card-border);
        margin: 14px 0;
    }

    /* Inline spinner (e.g. "FreeCode is thinking…") */
    .spinner {
        width: 12px;
        height: 12px;
        border: 2px solid rgba(128, 128, 128, 0.3);
        border-radius: 50%;
        border-top-color: var(--focus-border);
        animation: spin 0.8s linear infinite;
        display: inline-block;
        vertical-align: middle;
    }

    /* Bottom Sticky Panel */
    .bottom-panel {
        padding: 10px 14px 14px 14px;
        border-top: 1px solid var(--card-border);
        background: var(--bg-color);
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    /* --- Write scope bar -------------------------------------------------
       Deliberately quiet: it is reference information, not a notification. The only
       animation is a row lighting up when that location is written during a turn. */
    .scope-bar {
        border: 1px solid var(--card-border);
        border-radius: 6px;
        background: var(--input-bg);
        font-size: 10.5px;
        overflow: hidden;
    }

    .scope-summary {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 4px 8px;
        cursor: pointer;
        color: var(--text-muted);
        user-select: none;
    }
    .scope-summary:hover { color: var(--text-main); }

    .scope-icon { opacity: 0.7; }
    .scope-summary-text { flex-grow: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .scope-summary-text strong { color: var(--text-main); font-weight: 600; }
    .scope-chevron { opacity: 0.6; font-size: 9px; }

    /* Anything outside the open repository is the point of this bar — say it in the summary. */
    .scope-outside-count {
        color: var(--vscode-editorWarning-foreground, #cca700);
        font-weight: 600;
    }

    .scope-detail { display: none; padding: 2px 8px 6px 8px; }
    .scope-bar.open .scope-detail { display: block; }

    .scope-row {
        display: grid;
        grid-template-columns: 8px minmax(0, 1fr) auto;
        grid-template-areas: "dot path access" ".  note note";
        gap: 2px 6px;
        padding: 3px 0;
        align-items: center;
        border-top: 1px solid color-mix(in srgb, var(--card-border) 60%, transparent);
    }
    .scope-row:first-child { border-top: none; }

    .scope-dot {
        grid-area: dot;
        width: 6px; height: 6px; border-radius: 50%;
        background: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
    }
    /* Outside the repo reads differently at a glance — that is the whole signal. */
    .scope-row.outside .scope-dot { background: var(--vscode-editorWarning-foreground, #cca700); }
    .scope-row.excluded .scope-dot { background: var(--text-muted); opacity: 0.5; }

    .scope-path {
        grid-area: path;
        color: var(--text-main);
        font-family: var(--mono-font), monospace;
        font-size: 10px;
        overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    }
    .scope-access {
        grid-area: access;
        font-size: 8.5px;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        padding: 1px 4px;
        border-radius: 3px;
        background: var(--btn-secondary-bg);
        color: var(--text-muted);
    }
    .scope-access.rw { color: var(--vscode-editorWarning-foreground, #cca700); }
    .scope-note {
        grid-area: note;
        color: var(--text-muted);
        font-size: 9.5px;
        line-height: 1.3;
    }

    /* The single moving element: this location was just written. */
    .scope-row.written .scope-dot {
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--btn-primary-bg) 35%, transparent);
    }
    .scope-bar.active-write { border-color: var(--focus-border); }
    @media (prefers-reduced-motion: reduce) {
        .scope-row.written .scope-dot { box-shadow: none; outline: 2px solid var(--focus-border); }
    }

    /* --- Compose row: mode picker + clear -------------------------------- */
    .compose-row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
    }

    .mode-picker { position: relative; }

    .mode-trigger {
        display: flex;
        align-items: center;
        gap: 5px;
        background: transparent;
        border: 1px solid transparent;
        border-radius: 6px;
        color: var(--text-muted);
        font-family: var(--font-family);
        font-size: 10.5px;
        font-weight: 600;
        padding: 3px 7px;
        cursor: pointer;
    }
    .mode-trigger:hover { background: var(--btn-secondary-hover); color: var(--text-main); }
    .mode-trigger-glyph { opacity: 0.6; font-size: 10px; }
    .mode-trigger-caret { opacity: 0.6; font-size: 8px; }
    /* Auto applies edits without asking. That must be legible without reading the label. */
    .mode-trigger[data-mode="auto"] {
        color: var(--vscode-editorWarning-foreground, #cca700);
        border-color: color-mix(in srgb, var(--vscode-editorWarning-foreground, #cca700) 40%, transparent);
    }

    .mode-menu {
        display: none;
        position: absolute;
        bottom: calc(100% + 4px);
        left: 0;
        min-width: 210px;
        z-index: 90;
        background: var(--vscode-menu-background, var(--input-bg));
        border: 1px solid var(--card-border);
        border-radius: 6px;
        padding: 3px;
        box-shadow: 0 4px 14px rgba(0,0,0,0.28);
    }
    .mode-menu.open { display: block; }

    .mode-option {
        display: grid;
        grid-template-columns: 12px auto;
        grid-template-areas: "dot name" ".  desc";
        gap: 0 6px;
        width: 100%;
        text-align: left;
        background: transparent;
        border: none;
        border-radius: 4px;
        padding: 5px 7px;
        cursor: pointer;
        font-family: var(--font-family);
    }
    .mode-option:hover { background: var(--btn-secondary-hover); }
    .mode-option-dot {
        grid-area: dot;
        width: 6px; height: 6px; margin-top: 4px; border-radius: 50%;
        background: transparent;
        border: 1px solid var(--text-muted);
    }
    .mode-option.active .mode-option-dot { background: var(--btn-primary-bg); border-color: var(--btn-primary-bg); }
    .mode-option-name { grid-area: name; color: var(--text-main); font-size: 11px; font-weight: 600; }
    .mode-option.active .mode-option-name { color: var(--btn-primary-bg); }
    .mode-option-desc { grid-area: desc; color: var(--text-muted); font-size: 9.5px; }

    .compose-clear {
        background: transparent;
        border: none;
        color: var(--text-muted);
        font-family: var(--font-family);
        font-size: 10.5px;
        padding: 3px 6px;
        border-radius: 4px;
        cursor: pointer;
    }
    .compose-clear:hover { color: var(--text-main); background: var(--btn-secondary-hover); }

    .input-container {
        display: flex;
        align-items: flex-end;
        background: var(--input-bg);
        border: 1px solid var(--input-border);
        border-radius: 8px;
        padding: 6px;
        transition: border-color 0.2s ease, box-shadow 0.2s ease;
    }

    .input-container:focus-within {
        border-color: var(--focus-border);
        box-shadow: 0 0 0 2px color-mix(in srgb, var(--focus-border) 25%, transparent);
    }

    .chat-input {
        flex-grow: 1;
        background: transparent;
        border: none;
        color: var(--input-fg);
        font-family: var(--font-family);
        font-size: 12.5px;
        resize: none;
        max-height: 100px;
        height: 18px;
        outline: none;
        padding: 2px 6px;
        line-height: 1.4;
    }

    .send-btn {
        background: var(--btn-primary-bg);
        color: var(--btn-primary-fg);
        border: none;
        width: 24px;
        height: 24px;
        border-radius: 6px;
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        transition: background 0.2s ease;
    }

    .send-btn:hover {
        background: var(--btn-primary-hover);
    }
    .send-btn:active {
        background: color-mix(in srgb, var(--btn-primary-bg) 80%, black);
    }

    .send-btn:disabled {
        background: var(--btn-secondary-bg);
        color: var(--text-muted);
        cursor: not-allowed;
    }

    .stop-btn {
        background: var(--vscode-errorForeground, #ef4444);
        color: #ffffff;
        border: none;
        width: 24px;
        height: 24px;
        border-radius: 6px;
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        transition: background 0.2s ease;
    }

    .stop-btn:hover {
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 80%, black);
    }

    .stop-btn:active {
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 60%, black);
    }

    .icon {
        width: 12px;
        height: 12px;
        fill: currentColor;
    }

    @keyframes spin {
        to { transform: rotate(360deg); }
    }

    /* Harness Observability CSS */
    .harness-row {
        display: flex;
        justify-content: space-between;
        align-items: stretch;
        padding: 6px 12px;
        font-size: 11px;
        font-family: var(--mono-font), monospace;
        border-bottom: 1px solid var(--card-border);
        background: color-mix(in srgb, var(--card-bg) 50%, transparent);
        gap: 8px;
    }

    .harness-column {
        display: flex;
        flex-direction: column;
        justify-content: space-between;
        gap: 4px;
        flex: 1;
    }

    .harness-column-actions {
        display: flex;
        flex-direction: column;
        justify-content: space-between;
        align-items: flex-end;
        gap: 4px;
    }

    .harness-item {
        display: flex;
        justify-content: flex-start;
        align-items: center;
        gap: 6px;
        cursor: help;
    }

    .harness-column:nth-child(1) .harness-label {
        width: 48px;
        display: inline-block;
        color: var(--text-muted);
    }

    .harness-column:nth-child(2) .harness-label {
        width: 35px;
        display: inline-block;
        color: var(--text-muted);
    }

    .harness-val {
        font-family: var(--mono-font), monospace;
        color: var(--text-main);
    }
    .harness-val.positive {
        color: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
    }
    .harness-val.negative {
        color: var(--vscode-errorForeground, #ef4444);
    }
    
    /* Premium Timeline */
    .timeline-container {
        display: flex;
        flex-direction: column;
        gap: 12px;
        margin: 12px 0;
        padding: 0 4px;
        position: relative;
        width: 100%;
    }
    .timeline-container::before {
        content: '';
        position: absolute;
        left: 14px;
        top: 10px;
        bottom: 10px;
        width: 2px;
        background: var(--card-border);
        z-index: 0;
    }
    .timeline-item {
        display: flex;
        gap: 12px;
        position: relative;
        z-index: 1;
        animation: fadeIn 0.2s ease-out;
        width: 100%;
    }
    .timeline-badge {
        width: 22px;
        height: 22px;
        border-radius: 50%;
        background: var(--bg-color);
        border: 2px solid var(--card-border);
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 11px;
        font-weight: bold;
        color: var(--text-muted);
        flex-shrink: 0;
        box-shadow: 0 1px 3px rgba(0,0,0,0.1);
        transition: all 0.2s ease;
        z-index: 2;
    }
    .timeline-badge.running {
        border-color: var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d);
        color: var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d);
        background: color-mix(in srgb, var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d) 10%, var(--bg-color));
        animation: pulse-orange 1.5s infinite;
    }
    .timeline-badge.success {
        border-color: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
        color: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
        background: color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 10%, var(--bg-color));
    }
    .timeline-badge.error {
        border-color: var(--vscode-errorForeground, #ef4444);
        color: var(--vscode-errorForeground, #ef4444);
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 10%, var(--bg-color));
    }
    .timeline-badge.info {
        border-color: var(--focus-border);
        color: var(--focus-border);
        background: color-mix(in srgb, var(--focus-border) 10%, var(--bg-color));
    }
    .timeline-badge.proposal {
        border-color: var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d);
        color: var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d);
        background: color-mix(in srgb, var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d) 5%, var(--bg-color));
    }
    .timeline-content {
        flex-grow: 1;
        background: var(--card-bg);
        border: 1px solid var(--card-border);
        border-radius: 8px;
        padding: 8px 12px;
        box-shadow: 0 2px 6px rgba(0,0,0,0.05);
        backdrop-filter: blur(4px);
        -webkit-backdrop-filter: blur(4px);
        min-width: 0; /* Prevents overflow */
    }
    .timeline-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        font-weight: 600;
        font-size: 11.5px;
        color: var(--text-main);
        gap: 8px;
    }
    .timeline-title-text {
        display: flex;
        align-items: center;
        gap: 6px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .timeline-details {
        margin-top: 6px;
        font-size: 11px;
        color: var(--text-muted);
    }
    
    @keyframes pulse-orange {
        0% { box-shadow: 0 0 0 0 rgba(226, 192, 141, 0.4); }
        70% { box-shadow: 0 0 0 6px rgba(226, 192, 141, 0); }
        100% { box-shadow: 0 0 0 0 rgba(226, 192, 141, 0); }
    }

    /* Gate Verdicts */
    .gate-verdict-card {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 6px 10px;
        border-radius: 6px;
        margin: 4px 0;
        border: 1px solid var(--card-border);
        font-size: 11px;
        background: var(--input-bg);
        box-shadow: 0 1px 3px rgba(0,0,0,0.05);
        width: 100%;
        gap: 8px;
    }
    .gate-verdict-card.passed {
        border-left: 4px solid var(--vscode-gitDecoration-addedResourceForeground, #10b981);
        background: color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 5%, var(--input-bg));
    }
    .gate-verdict-card.failed {
        border-left: 4px solid var(--vscode-errorForeground, #ef4444);
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 5%, var(--input-bg));
    }
    .gate-badge {
        padding: 1px 6px;
        border-radius: 12px;
        font-size: 9px;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.3px;
        flex-shrink: 0;
    }
    .gate-badge.passed {
        background: color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 15%, transparent);
        color: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
    }
    .gate-badge.failed {
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 15%, transparent);
        color: var(--vscode-errorForeground, #ef4444);
    }
    .gate-verdict-card.warn {
        border-left: 4px solid var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d);
        background: color-mix(in srgb, var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d) 6%, var(--input-bg));
    }
    .gate-badge.warn {
        background: color-mix(in srgb, var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d) 18%, transparent);
        color: var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d);
    }
    .gate-reasons {
        margin: 2px 0 6px 0;
        padding-left: 18px;
        font-size: 10px;
        color: var(--text-muted);
        list-style: disc;
    }
    .gate-reasons li { margin: 1px 0; word-break: break-word; }

    /* Files Read List */
    .files-read-container {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        margin: 6px 0;
        width: 100%;
    }
    .file-read-tag {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        font-size: 10.5px;
        background: var(--input-bg);
        border: 1px solid var(--card-border);
        color: var(--vscode-textLink-foreground, #3794ff);
        padding: 2px 6px;
        border-radius: 4px;
        cursor: pointer;
        font-family: var(--mono-font), monospace;
        transition: all 0.15s ease;
    }
    .file-read-tag:hover {
        border-color: var(--vscode-textLink-foreground, #3794ff);
        background: color-mix(in srgb, var(--vscode-textLink-foreground, #3794ff) 8%, var(--input-bg));
    }
    .file-read-icon {
        font-size: 10px;
        color: var(--text-muted);
    }

    /* Beautiful Diff Views */
    .diff-wrapper {
        margin-top: 8px;
        border: 1px solid var(--card-border);
        border-radius: 6px;
        overflow: hidden;
        font-family: var(--mono-font), monospace;
        font-size: 11px;
        background: var(--console-bg);
        max-height: 250px;
        overflow-y: auto;
        width: 100%;
        text-align: left;
    }
    .proposal-edit-textarea {
        width: 100%;
        min-height: 120px;
        background: var(--input-bg);
        color: var(--text-main);
        border: 1px solid var(--card-border);
        font-family: var(--mono-font), monospace;
        font-size: 11px;
        padding: 6px;
        box-sizing: border-box;
        border-radius: 4px;
        resize: vertical;
        margin-top: 4px;
    }
    .edit-mode-toggle {
        font-size: 9px;
        background: var(--btn-secondary-bg);
        color: var(--text-muted);
        border: 1px solid var(--card-border);
        padding: 2px 5px;
        border-radius: 3px;
        cursor: pointer;
        user-select: none;
        transition: all 0.2s ease;
    }
    .edit-mode-toggle:hover {
        background: var(--btn-secondary-hover);
        color: var(--text-main);
    }
    .recovery-retry-container {
        display: flex;
        gap: 6px;
        margin-top: 8px;
        border-top: 1px solid var(--card-border);
        padding-top: 8px;
        width: 100%;
    }
    .recovery-input {
        flex: 1;
        background: var(--input-bg);
        color: var(--text-main);
        border: 1px solid var(--card-border);
        border-radius: 4px;
        padding: 4px 8px;
        font-size: 11px;
        box-sizing: border-box;
    }
    .recovery-btn {
        background: var(--btn-primary-bg);
        color: var(--btn-primary-fg);
        border: none;
        border-radius: 4px;
        padding: 4px 10px;
        font-size: 11px;
        font-weight: 600;
        cursor: pointer;
        transition: background 0.2s;
    }
    .recovery-btn:hover {
        background: var(--focus-border);
    }
    .diff-line {
        display: flex;
        white-space: pre-wrap;
        word-wrap: break-word;
        padding: 2px 8px;
        line-height: 1.4;
    }
    .diff-line.added {
        background-color: rgba(16, 185, 129, 0.15);
        color: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
        border-left: 3px solid var(--vscode-gitDecoration-addedResourceForeground, #10b981);
    }
    .diff-line.removed {
        background-color: rgba(239, 68, 68, 0.15);
        color: var(--vscode-errorForeground, #ef4444);
        border-left: 3px solid var(--vscode-errorForeground, #ef4444);
    }
    .diff-line.unchanged {
        color: var(--text-muted);
        opacity: 0.85;
        padding-left: 11px;
    }
    
    /* Inline Action buttons for proposals */
    .proposal-actions {
        display: flex;
        gap: 8px;
        margin-top: 8px;
        width: 100%;
    }
    .proposal-btn {
        padding: 4px 12px;
        border-radius: 4px;
        font-size: 10.5px;
        font-weight: 600;
        cursor: pointer;
        border: 1px solid var(--card-border);
        transition: all 0.2s ease;
    }
    .proposal-btn.accept {
        background: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
        color: #ffffff;
        border-color: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
    }
    .proposal-btn.accept:hover {
        background: color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 85%, black);
    }
    .proposal-btn.discard {
        background: var(--vscode-errorForeground, #ef4444);
        color: #ffffff;
        border-color: var(--vscode-errorForeground, #ef4444);
    }
    .proposal-btn.discard:hover {
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 85%, black);
    }
    .proposal-status-badge {
        display: inline-block;
        font-size: 9px;
        font-weight: 700;
        padding: 1px 5px;
        border-radius: 3px;
        text-transform: uppercase;
        letter-spacing: 0.3px;
        border: 1px solid currentColor;
        flex-shrink: 0;
    }
    .proposal-status-badge.pending {
        color: var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d);
        background: color-mix(in srgb, var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d) 10%, transparent);
    }
    .proposal-status-badge.applied {
        color: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
        background: color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 10%, transparent);
    }
    .proposal-status-badge.discarded {
        color: var(--vscode-errorForeground, #ef4444);
        background: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 10%, transparent);
    }

    /* PIC-10 — compact horizontal pipeline strip (additive; mode = fc-compact | fc-vertical | fc-hidden) */
    .fc-strip { display: none; padding: 5px 4px; }
    .timeline-container.fc-compact > .fc-strip { display: block; }
    .timeline-container.fc-compact > .timeline-item { display: none; }
    .timeline-container.fc-compact.fc-expanded > .timeline-item { display: flex; }
    .timeline-container.fc-vertical > .fc-strip { display: none; }
    .timeline-container.fc-hidden > * { display: none; }
    .fc-pipe { display: flex; align-items: center; flex-wrap: wrap; row-gap: 4px; }
    .fc-chip {
        display: inline-flex; align-items: center; gap: 4px;
        font-size: 11px; line-height: 1.5; padding: 1px 8px; border-radius: 10px;
        border: 1px solid var(--vscode-panel-border, rgba(128,128,128,0.3));
        color: var(--text-muted); white-space: nowrap;
    }
    .fc-chip.fc-pend { border-style: dashed; opacity: 0.55; }
    .fc-chip.fc-active { border-color: var(--vscode-focusBorder, #007acc); color: var(--vscode-foreground); }
    .fc-chip.fc-gate {
        color: var(--vscode-gitDecoration-addedResourceForeground, #10b981);
        border-color: color-mix(in srgb, var(--vscode-gitDecoration-addedResourceForeground, #10b981) 45%, transparent);
        font-weight: 600;
    }
    .fc-chip.fc-bad {
        color: var(--vscode-errorForeground, #ef4444);
        border-color: color-mix(in srgb, var(--vscode-errorForeground, #ef4444) 45%, transparent);
        font-weight: 600;
    }
    .fc-ln { width: 12px; height: 0; border-top: 1px solid var(--vscode-panel-border, rgba(128,128,128,0.3)); margin: 0 1px; flex: none; }
    .fc-meta { font-size: 11px; color: var(--text-muted); margin-left: 8px; }
    .fc-exp { font-size: 10px; color: var(--text-muted); background: transparent; border: 1px solid var(--vscode-panel-border, rgba(128,128,128,0.3)); border-radius: 4px; padding: 1px 6px; margin-left: 8px; cursor: pointer; }
    .fc-exp:hover { color: var(--vscode-foreground); }
    .fc-why { font-size: 11px; color: var(--vscode-errorForeground, #ef4444); margin-top: 4px; padding-left: 4px; }
    `;
}
