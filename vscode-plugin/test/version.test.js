// The version handshake between extension and daemon.
//
// `src/version.ts` imports nothing — no `vscode`, no bundler-injected module — so it can be
// compiled in isolation with the TypeScript compiler and required directly. That is why the
// function lives there instead of inside provider.ts: a check nobody can test is a check nobody
// can trust.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

/** Compile src/version.ts on its own and load it. */
function loadVersionModule() {
    const out = fs.mkdtempSync(path.join(os.tmpdir(), 'freecode-version-'));
    execFileSync(
        process.execPath,
        [
            path.join(__dirname, '..', 'node_modules', 'typescript', 'bin', 'tsc'),
            path.join(__dirname, '..', 'src', 'version.ts'),
            '--outDir', out,
            '--module', 'commonjs',
            '--target', 'es2020',
        ],
        { stdio: 'pipe' },
    );
    return require(path.join(out, 'version.js'));
}

const { describeVersionSkew } = loadVersionModule();

test('matching minor versions produce no warning', () => {
    assert.equal(describeVersionSkew('0.1.5', '0.1.5'), null);
});

test('a patch difference is not a mismatch — the contract did not change', () => {
    assert.equal(describeVersionSkew('0.1.9', '0.1.2'), null);
    assert.equal(describeVersionSkew('1.4.0', '1.4.99'), null);
});

test('a minor difference IS a mismatch, and names both versions', () => {
    const msg = describeVersionSkew('0.1.0', '0.2.0');
    assert.ok(msg, 'expected a warning');
    assert.match(msg, /0\.1\.0/);
    assert.match(msg, /0\.2\.0/);
    assert.match(msg, /cargo build --release -p freecode-daemon/, 'must name the fix');
});

test('a major difference is a mismatch', () => {
    assert.ok(describeVersionSkew('1.0.0', '2.0.0'));
});

test('the exact regression this exists for: a stale daemon left behind', () => {
    // The daemon reported a hardcoded "0.1.0" for four releases while the extension moved on.
    // That is the case that used to be silent.
    const msg = describeVersionSkew('0.1.0', '0.5.0');
    assert.ok(msg, 'a daemon four minors behind must be reported');
});

test('an unparseable version never cries wolf', () => {
    // A fork, a dev build, an empty string, a daemon that answered with something unexpected:
    // none of these are evidence of a mismatch, and a false alarm teaches people to ignore it.
    for (const bad of ['', '   ', 'dev', 'unknown', 'v-broken', null]) {
        assert.equal(describeVersionSkew(bad, '0.1.5'), null, `cried wolf on daemon ${JSON.stringify(bad)}`);
        assert.equal(describeVersionSkew('0.1.5', bad), null, `cried wolf on extension ${JSON.stringify(bad)}`);
    }
    // `undefined` is excluded on purpose: for the second parameter it means "use the default",
    // which is the real extension version — not an unparseable input.
    assert.equal(describeVersionSkew(undefined, '0.1.5'), null);
});

test('pre-release and build suffixes compare on major.minor alone', () => {
    assert.equal(describeVersionSkew('0.1.5-rc1', '0.1.5'), null);
    assert.equal(describeVersionSkew('0.1.5+build.7', '0.1.5'), null);
    assert.ok(describeVersionSkew('0.2.0-rc1', '0.1.5'), 'a real minor gap still reports');
});

test('two-component versions are handled', () => {
    assert.equal(describeVersionSkew('1.2', '1.2'), null);
    assert.ok(describeVersionSkew('1.2', '1.3'));
});

test('the shipped manifest version is parseable by this check', () => {
    // A guard against the manifest drifting to something the comparison silently ignores.
    const pkg = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8'));
    assert.match(pkg.version, /^\d+\.\d+/, `manifest version ${pkg.version} would never compare`);
    assert.equal(describeVersionSkew(pkg.version, pkg.version), null);
});
