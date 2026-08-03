// Version handshake between the two halves.
//
// Deliberately free of any `vscode` import: that keeps it a pure module a test can load without
// an editor host, which is the whole reason it lives here rather than inside provider.ts.

/** Injected by esbuild from package.json (see `define` in esbuild.js). The fallback only applies
 *  outside a bundle — i.e. under test. */
declare const __EXTENSION_VERSION__: string | undefined;

export const EXTENSION_VERSION: string =
    typeof __EXTENSION_VERSION__ === 'string' ? __EXTENSION_VERSION__ : '0.0.0-dev';

/**
 * Compare this extension against the daemon it just reached; describe the problem when the two do
 * not belong together, or return `null` when they match.
 *
 * The extension and the daemon are installed and updated separately but speak one gRPC contract,
 * so they ship as a pair and are versioned in lockstep. A mismatch used to be invisible: the
 * daemon reported its version, the panel printed it, and nothing compared them — so a stale
 * daemon surfaced much later as a field that silently was not there.
 *
 * MAJOR.MINOR only. Patch releases do not change the contract, and warning about them would train
 * people to dismiss the warning that matters.
 *
 * An unparseable version is not evidence of a mismatch: say nothing rather than cry wolf. A
 * daemon built from a fork, or a dev build with a `-dev` suffix, must not produce a false alarm.
 */
export function describeVersionSkew(
    daemonVersion: string,
    extensionVersion: string = EXTENSION_VERSION,
): string | null {
    const line = (v: string): string | null => {
        const m = /^(\d+)\.(\d+)(?:\D|$)/.exec((v ?? '').trim());
        return m ? `${m[1]}.${m[2]}` : null;
    };
    const ext = line(extensionVersion);
    const dae = line(daemonVersion);
    if (!ext || !dae || ext === dae) {
        return null;
    }
    return (
        `Version mismatch: extension ${extensionVersion}, daemon ${daemonVersion}. ` +
        'They share one gRPC contract and ship together, so a mismatched pair can fail in ways ' +
        'that look like a bug. Rebuild the daemon (`cargo build --release -p freecode-daemon`) ' +
        'or install the extension from the matching release.'
    );
}
