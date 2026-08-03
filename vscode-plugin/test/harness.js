// Test harness for the webview's pure functions.
//
// The problem: `escapeHtml` and `parseMarkdown` are the boundary between model-controlled
// text and the panel's DOM — the highest-value code in the extension to test — and
// `FC_ACTIONS` is the whitelist that decides what markup is allowed to invoke. All of it
// lives INSIDE the template literal returned by `getWebviewJs()`: text, not exported
// symbols, so nothing can `import` it.
//
// The way in is the one esbuild.js already uses for its syntax guard: bundle the module,
// then evaluate the string it returns. Evaluating it means the top-level statements run too
// (`acquireVsCodeApi()`, `addEventListener`, `setInterval`, …), none of which exist in Node —
// hence the stubs below. Everything stubbed here is inert: the functions under test are pure
// string→string and touch none of it.

const esbuild = require('esbuild');
const fs = require('fs');
const path = require('path');
const os = require('os');

/** Bundle src/webview/client.ts and return its `getWebviewJs()` output. */
function buildWebviewSource() {
    const out = path.join(os.tmpdir(), `freecode_webview_test_${process.pid}.cjs`);
    try {
        esbuild.buildSync({
            entryPoints: [path.join(__dirname, '..', 'src', 'webview', 'client.ts')],
            bundle: true,
            format: 'cjs',
            platform: 'node',
            outfile: out,
            logLevel: 'silent',
        });
        return require(out).getWebviewJs();
    } finally {
        try { fs.unlinkSync(out); } catch { /* temp file; nothing depends on its removal */ }
    }
}

/** A DOM element stub that absorbs every access the top-level init code makes. */
function stubElement() {
    const el = {
        addEventListener() {},
        appendChild() {},
        removeChild() {},
        setAttribute() {},
        getAttribute() { return null; },
        focus() {},
        scrollIntoView() {},
        closest() { return null; },
        querySelector() { return null; },
        querySelectorAll() { return []; },
        classList: { add() {}, remove() {}, toggle() {}, contains() { return false; } },
        style: {},
        dataset: {},
        children: [],
        value: '',
        innerHTML: '',
        innerText: '',
        textContent: '',
        scrollTop: 0,
        scrollHeight: 0,
    };
    return el;
}

/**
 * Evaluate the webview script under stubs and hand back the pure functions.
 * `new Function` (not `eval`) so the body sees only the parameters we pass.
 */
function loadWebviewFunctions() {
    const source = buildWebviewSource();
    const doc = {
        getElementById: () => stubElement(),
        querySelector: () => stubElement(),
        querySelectorAll: () => [],
        createElement: () => stubElement(),
        addEventListener() {},
        body: stubElement(),
    };
    const win = { addEventListener() {}, postMessage() {} };

    const factory = new Function(
        'acquireVsCodeApi', 'document', 'window', 'setInterval', 'setTimeout',
        'clearInterval', 'clearTimeout', 'localStorage', 'console',
        `${source}
         return { escapeHtml, parseMarkdown, FC_ACTIONS, fcDispatch };`
    );

    return factory(
        () => ({ postMessage() {}, getState() { return undefined; }, setState() {} }),
        doc,
        win,
        () => 0,   // setInterval: a real timer would keep the test process alive
        () => 0,   // setTimeout
        () => {},
        () => {},
        { getItem: () => null, setItem() {}, removeItem() {} },
        { log() {}, warn() {}, error() {} },
    );
}

module.exports = { loadWebviewFunctions };
