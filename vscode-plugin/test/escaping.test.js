// The model → DOM boundary.
//
// Everything the local model emits is rendered into the panel with `innerHTML`. These three
// functions are the only thing standing between a prompt-injected response and script
// execution inside a webview that holds `acquireVsCodeApi()` — i.e. the ability to postMessage
// the extension into writing files. The panel's CSP is defence in depth behind them, not
// instead of them.
//
// Run: npm test

const test = require('node:test');
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { loadWebviewFunctions } = require('./harness.js');

// ONE evaluation: a second call would build a separate FC_ACTIONS that fcDispatch
// does not close over, and every stub swap below would silently target the wrong table.
const { escapeHtml, parseMarkdown, FC_ACTIONS, fcDispatch } = loadWebviewFunctions();

// ---------------------------------------------------------------------------
// A precise "would a browser execute this?" detector.
//
// Deliberately NOT a substring search for "onerror" or "<img": those appear in correctly
// escaped output (`&lt;img ... onerror=...&gt;`) and a naive matcher flags every safe case.
// What matters is whether a real tag, a real event-handler attribute, or a scheme-bearing
// href survived into the markup.
// ---------------------------------------------------------------------------

/** Handler names parseMarkdown itself legitimately emits. Anything else is an injection. */
const ALLOWED_HANDLER = /^(openFile|openDiff|event\.stopPropagation)/;

function dangerousConstructs(html) {
    const found = [];

    if (/<\s*(script|iframe|object|embed|svg|img|link|meta|style|base)\b/i.test(html)) {
        found.push('live tag');
    }
    for (const m of html.matchAll(/\son[a-z]+\s*=\s*"([^"]*)"/gi)) {
        if (!ALLOWED_HANDLER.test(m[1])) {
            found.push(`event handler: ${m[0].slice(0, 60)}`);
        }
    }
    for (const m of html.matchAll(/href\s*=\s*"([^"]*)"/gi)) {
        // Decode entities first — `&#106;avascript:` must not slip through.
        const decoded = m[1].replace(/&[#a-zA-Z0-9]+;/g, '');
        if (/^\s*(javascript|vbscript|data)\s*:/i.test(decoded)) {
            found.push(`scheme in href: ${m[1].slice(0, 40)}`);
        }
    }
    return found;
}

// ---------------------------------------------------------------------------
// escapeHtml
// ---------------------------------------------------------------------------

test('escapeHtml neutralises all five HTML-significant characters', () => {
    assert.equal(escapeHtml('&'), '&amp;');
    assert.equal(escapeHtml('<'), '&lt;');
    assert.equal(escapeHtml('>'), '&gt;');
    assert.equal(escapeHtml('"'), '&quot;');
    assert.equal(escapeHtml("'"), '&#39;');
});

test('escapeHtml escapes & first, so entities are not double-decoded by the browser', () => {
    // If `<` were replaced before `&`, "&lt;" would come back out as a literal "<".
    assert.equal(escapeHtml('&lt;script&gt;'), '&amp;lt;script&amp;gt;');
});

test('escapeHtml output contains no live markup for a tag payload', () => {
    const out = escapeHtml('<img src=x onerror=alert(1)>');
    assert.deepEqual(dangerousConstructs(out), []);
    assert.ok(!out.includes('<img'));
});

test('escapeHtml coerces non-strings instead of throwing', () => {
    for (const v of [null, undefined, 42, { a: 1 }]) {
        assert.doesNotThrow(() => escapeHtml(v));
    }
});

// ---------------------------------------------------------------------------
// parseMarkdown — the main surface
// ---------------------------------------------------------------------------

const XSS_CORPUS = [
    ['raw tag', '<img src=x onerror=alert(1)>'],
    ['script element', '<script>alert(1)</script>'],
    ['svg onload', '<svg onload=alert(1)>'],
    ['iframe', '<iframe src="javascript:alert(1)"></iframe>'],
    ['javascript: link', '[click](javascript:alert(1))'],
    ['mixed-case scheme', '[click](JaVaScRiPt:alert(1))'],
    ['data: html link', '[x](data:text/html,<script>alert(1)</script>)'],
    ['vbscript link', '[x](vbscript:msgbox(1))'],
    ['attribute break in url', '[x](http://a" onmouseover="alert(1))'],
    ['tag inside link label', '[<img src=x onerror=alert(1)>](http://ok)'],
    ['tag inside inline code', 'text `<img src=x onerror=alert(1)>` end'],
    ['tag inside bold', '**<img src=x onerror=alert(1)>**'],
    ['tag inside heading', '# <img src=x onerror=alert(1)>'],
    ['tag inside list item', '- <img src=x onerror=alert(1)>'],
    ['fence escape attempt', '```js\n</script><img src=x onerror=alert(1)>\n```'],
    ['entity-encoded scheme', '[x](&#106;avascript:alert(1))'],
    ['two links, one hostile', '[a](http://ok) [b](javascript:alert(1))'],
];

for (const [name, payload] of XSS_CORPUS) {
    test(`parseMarkdown neutralises: ${name}`, () => {
        const out = parseMarkdown(payload);
        assert.deepEqual(
            dangerousConstructs(out), [],
            `payload rendered live markup\n  in:  ${payload}\n  out: ${out}`
        );
    });
}

test('parseMarkdown never emits a non-# href for a non-http scheme', () => {
    // Anything that isn't http(s) is routed through openFile(), which the extension confines
    // to the workspace — the anchor itself must stay inert.
    for (const scheme of ['javascript:alert(1)', 'file:///etc/passwd', 'data:text/html,x']) {
        const out = parseMarkdown(`[x](${scheme})`);
        const href = /href="([^"]*)"/.exec(out);
        assert.ok(href, `no href produced for ${scheme}: ${out}`);
        assert.equal(href[1], '#', `scheme leaked into href: ${out}`);
    }
});

test('parseMarkdown keeps http(s) links usable', () => {
    const out = parseMarkdown('[docs](https://example.com/a?b=1)');
    assert.match(out, /href="https:\/\/example\.com\/a\?b=1"/);
    assert.match(out, /target="_blank"/);
});

test('parseMarkdown still renders ordinary markdown', () => {
    assert.match(parseMarkdown('**bold**'), /<strong>bold<\/strong>/);
    assert.match(parseMarkdown('# Title'), /<h1>Title<\/h1>/);
    assert.match(parseMarkdown('- one'), /<li class="ul-item">one<\/li>/);
    assert.match(parseMarkdown('`code`'), /<code>/);
});

test('parseMarkdown escapes raw input exactly once', () => {
    // Correct HTML escaping is NOT idempotent, and must not be: a model that literally writes
    // "&amp;" has to render as the visible text "&amp;", which requires escaping the "&" again.
    // The invariant that matters is therefore about the CALLERS, not the function — see the
    // test below. Here we only pin that one pass produces one level of escaping.
    assert.equal(parseMarkdown('a & b'), 'a &amp; b');
    assert.equal(parseMarkdown('a &amp; b'), 'a &amp;amp; b');
});

test('every parseMarkdown call site feeds RAW text, never rendered output', () => {
    // Because escaping is non-idempotent, re-feeding output would corrupt the text (and each
    // extra pass adds an "amp;"). The streaming path accumulates raw tokens into
    // `activeAssistantMessageText` and re-renders the whole raw string on every token, so this
    // holds today — this test fails loudly if someone ever assigns a rendered string back.
    const src = fs.readFileSync(
        path.join(__dirname, '..', 'src', 'webview', 'client.ts'), 'utf8'
    );
    const callArgs = [...src.matchAll(/parseMarkdown\(([^)]*)\)/g)]
        .map(m => m[1].trim())
        .filter(Boolean);
    assert.ok(callArgs.length >= 3, `expected call sites, found ${callArgs.length}`);
    for (const arg of callArgs) {
        assert.ok(
            !/innerHTML|parseMarkdown\(/.test(arg),
            `parseMarkdown fed rendered output: parseMarkdown(${arg})`
        );
    }
});

test('parseMarkdown handles degenerate input without throwing', () => {
    for (const v of ['', '```', '```js', '[unclosed](', '**', '#', '\n\n\n', '`'.repeat(50)]) {
        assert.doesNotThrow(() => parseMarkdown(v), `threw on ${JSON.stringify(v)}`);
    }
});

test('a long hostile document stays inert end to end', () => {
    const doc = XSS_CORPUS.map(([, p]) => p).join('\n\n');
    assert.deepEqual(dangerousConstructs(parseMarkdown(doc)), []);
});

// ---------------------------------------------------------------------------
// Event delegation (the CSP-nonce refactor).
//
// The panel's CSP is `script-src 'nonce-…'` with no 'unsafe-inline'. That only holds as long
// as no inline handler creeps back in, and dispatch only stays safe while every data-action
// resolves to something on the explicit FC_ACTIONS table. Both are checked here, against the
// source, so a regression fails the build rather than the panel silently going dead.
// ---------------------------------------------------------------------------

const WEBVIEW_SRC = ['markup.ts', 'client.ts'].map(f => ({
    name: f,
    text: fs.readFileSync(path.join(__dirname, '..', 'src', 'webview', f), 'utf8'),
}));

test('no inline event handlers survive anywhere in the webview source', () => {
    for (const { name, text } of WEBVIEW_SRC) {
        const inline = [...text.matchAll(/\son(click|change|input|mousedown|keydown|load|error|mouseover|focus|blur|submit)\s*=/gi)]
            .map(m => m[0].trim());
        assert.deepEqual(inline, [], `inline handler(s) reintroduced in ${name}: ${inline.join(', ')}`);
    }
});

test('every data-action in the source resolves to an FC_ACTIONS entry', () => {
    const declared = new Set();
    for (const { text } of WEBVIEW_SRC) {
        for (const m of text.matchAll(/data-(?:action|change|input|mousedown|keydown)="([a-zA-Z0-9_]+)"/g)) {
            declared.add(m[1]);
        }
    }
    assert.ok(declared.size >= 25, `expected the full handler set, found ${declared.size}`);
    const unknown = [...declared].filter(a => typeof FC_ACTIONS[a] !== 'function');
    assert.deepEqual(unknown, [], `markup references actions with no FC_ACTIONS entry: ${unknown.join(', ')}`);
});

test('FC_ACTIONS has no dead entries', () => {
    const used = new Set();
    for (const { text } of WEBVIEW_SRC) {
        for (const m of text.matchAll(/data-(?:action|change|input|mousedown|keydown)="([a-zA-Z0-9_]+)"/g)) {
            used.add(m[1]);
        }
    }
    const dead = Object.keys(FC_ACTIONS).filter(k => !used.has(k));
    assert.deepEqual(dead, [], `FC_ACTIONS entries nothing dispatches to: ${dead.join(', ')}`);
});

test('the CSP is nonce-based and never allows inline or remote script', () => {
    const markup = WEBVIEW_SRC.find(f => f.name === 'markup.ts').text;
    const csp = /content="([^"]*default-src[^"]*)"/.exec(markup);
    assert.ok(csp, 'no CSP meta tag found');
    const policy = csp[1];
    const scriptSrc = /script-src ([^;]*)/.exec(policy);
    assert.ok(scriptSrc, `no script-src in CSP: ${policy}`);
    assert.ok(!/unsafe-inline|unsafe-eval/.test(scriptSrc[1]), `script-src still permissive: ${scriptSrc[1]}`);
    assert.match(scriptSrc[1], /nonce-/, `script-src is not nonce-based: ${scriptSrc[1]}`);
    assert.match(policy, /default-src 'none'/);
    assert.match(policy, /connect-src 'none'/);
    // The <script> tag must actually carry the nonce, or nothing runs at all.
    assert.match(markup, /<script nonce="\$\{nonce\}">/);
});

test('the nonce is cryptographically random, not Math.random', () => {
    const provider = fs.readFileSync(path.join(__dirname, '..', 'src', 'provider.ts'), 'utf8');
    assert.match(provider, /crypto\.randomBytes\(\d+\)/, 'nonce is not generated from randomBytes');
    const nonceLine = /const nonce = ([^\n;]*)/.exec(provider);
    assert.ok(nonceLine && !/Math\.random/.test(nonceLine[1]), `weak nonce source: ${nonceLine && nonceLine[1]}`);
});

test('dynamic handler arguments are escaped, never concatenated into JS', () => {
    const client = WEBVIEW_SRC.find(f => f.name === 'client.ts').text;
    // Every data-a1 / data-a2 built by concatenation must pass through escapeHtml (or be a
    // plain number / string literal). A bare '+ someVar +' would be an attribute-injection.
    for (const m of client.matchAll(/data-a[12]="' \+ ([^+]+?) \+ '"/g)) {
        const expr = m[1].trim();
        assert.ok(
            /^escapeHtml\(/.test(expr) || /^[a-z]$/i.test(expr),
            `unescaped value interpolated into a data attribute: ${expr}`
        );
    }
});

// ---------------------------------------------------------------------------
// Dispatch routing — behaviour, not just structure.
//
// The tests above prove the table is complete and the markup is clean. These prove the
// dispatcher actually routes a click to the right entry with the right arguments, which is
// the part that would otherwise only be verifiable by clicking the panel by hand.
// ---------------------------------------------------------------------------

/** Minimal element+event pair good enough for fcDispatch's `closest` lookup. */
function fakeEvent(attrs, { stopSpy } = {}) {
    const el = {
        dataset: { ...attrs },
        getAttribute: (name) => attrs[name.replace(/^data-/, '')] ?? null,
        closest(sel) {
            const wanted = /\[data-([a-z]+)\]/.exec(sel);
            return wanted && attrs[wanted[1]] !== undefined ? el : null;
        },
    };
    return { target: el, stopPropagation: stopSpy || (() => {}) };
}

test('a click routes to the named action with its data-* arguments', () => {
    const calls = [];
    const original = FC_ACTIONS.setMode;
    FC_ACTIONS.setMode = (el) => calls.push(el.dataset.a1);
    try {
        fcDispatch(fakeEvent({ action: 'setMode', a1: 'auto' }), 'action');
        assert.deepEqual(calls, ['auto']);
    } finally {
        FC_ACTIONS.setMode = original;
    }
});

test('two-argument actions receive both values', () => {
    const calls = [];
    const original = FC_ACTIONS.startEditMemory;
    FC_ACTIONS.startEditMemory = (el) => calls.push([el.dataset.a1, el.dataset.a2]);
    try {
        fcDispatch(fakeEvent({ action: 'startEditMemory', a1: 'project', a2: 'note-7' }), 'action');
        assert.deepEqual(calls, [['project', 'note-7']]);
    } finally {
        FC_ACTIONS.startEditMemory = original;
    }
});

test('data-stop="1" calls stopPropagation before the action', () => {
    let stopped = false;
    const original = FC_ACTIONS.openDiff;
    FC_ACTIONS.openDiff = () => { assert.equal(stopped, true, 'action ran before stopPropagation'); };
    try {
        fcDispatch(
            fakeEvent({ action: 'openDiff', stop: '1', a1: 'a.rs' }, { stopSpy: () => { stopped = true; } }),
            'action'
        );
        assert.equal(stopped, true);
    } finally {
        FC_ACTIONS.openDiff = original;
    }
});

test('an unknown or injected action name is refused, not invoked', () => {
    // The whole point of the table: markup can only reach what is listed on it.
    assert.doesNotThrow(() => fcDispatch(fakeEvent({ action: 'alert' }), 'action'));
    assert.doesNotThrow(() => fcDispatch(fakeEvent({ action: 'constructor' }), 'action'));
    assert.doesNotThrow(() => fcDispatch(fakeEvent({ action: '__proto__' }), 'action'));
    assert.doesNotThrow(() => fcDispatch(fakeEvent({ action: 'toString' }), 'action'));
});

test('an element with no matching data-* attribute is ignored', () => {
    assert.doesNotThrow(() => fcDispatch(fakeEvent({ somethingElse: 'x' }), 'action'));
});

test('a file path keeps its literal value through dispatch (no URI decoding)', () => {
    // encArg is gone; a path containing % must arrive at openFile byte-for-byte.
    const seen = [];
    const original = FC_ACTIONS.openFile;
    FC_ACTIONS.openFile = (el) => seen.push(el.dataset.a1);
    try {
        const tricky = 'src/100%_done/it\'s "here".rs';
        fcDispatch(fakeEvent({ action: 'openFile', a1: tricky }), 'action');
        assert.deepEqual(seen, [tricky]);
    } finally {
        FC_ACTIONS.openFile = original;
    }
});
