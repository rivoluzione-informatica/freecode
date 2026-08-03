# Security policy

FreeCode edits files and executes commands on the machine it runs on. That makes its threat model
worth stating plainly, including the parts it does **not** defend.

## Reporting a vulnerability

Use GitHub's [private vulnerability reporting](https://github.com/rivoluzione-informatica/freecode/security/advisories/new)
(Security → Report a vulnerability). If that is unavailable, email **fabrizio.salmi@gmail.com**
with `[freecode security]` in the subject.

Please include what you would want to receive: the version or commit, the platform, a minimal
reproduction, and what an attacker gains. A working proof of concept is welcome but not required —
a precise description of the mechanism is worth more than a fragile exploit.

This is a small project maintained by one person, so no response-time commitment would be honest.
Expect an acknowledgement within a week. Nothing is published or fixed silently: the changelog
entry will say what was wrong.

Please do not open a public issue for something exploitable until there is a fix.

## Supported versions

The latest release, and `main`. There are no maintained release branches.

## Threat model

**What FreeCode assumes is trusted:** the operator, and the machine's other local processes.

**What it treats as untrusted:** everything the model emits, everything in the opened repository,
and everything already stored in memory files.

That second list is the design's centre of gravity. A model can be steered by content it reads, so
its output is never taken on trust — deterministic gates decide, and a hard veto cannot be
outvoted by model confidence.

### Defended

| Vector | Defence |
|---|---|
| **Writes outside the workspace** | `..` and absolute paths rejected, and the resolved path must land under the real root — the deepest existing ancestor is canonicalized, so a symlink pointing out is refused too |
| **Prompt injection via the prompt or a memory** | Pattern scan before the turn; a tripped memory is stripped rather than ingested |
| **Prompt injection via the repository's own system prompt** | `<workspace>/proto/system_prompt.md` passes the same Injection Gate; a tripped template is refused in favour of the built-in default, and the mode instruction is re-appended if the template dropped it |
| **Untrusted repo registering commands** | Analyzer definitions are read only from `~/.freecode/config.json`, never from the project |
| **Destructive or exfiltrating commands** | Default-deny policy: a command runs unattended only if provably read-only or a test; catastrophic and exfil patterns are refused unconditionally, with no approval path |
| **Auto-execution on the host** | Auto mode executes only inside an ephemeral `--network none` container. Without it, Auto refuses and defers to a human |
| **Executing untrusted build steps** | Verification is skipped for the turn when the model writes a build manifest or script |
| **Blast radius in Auto mode** | Dotfiles, CI workflows, dependency manifests, lockfiles, containerfiles and shell scripts are full-access paths and refused in Auto |
| **Secrets written into files** | Content scan blocks writes carrying known token shapes, private-key blocks, or high-entropy values bound to secret-like names |
| **Hidden instructions in written content** | Zero-width, bidi-override and homoglyph detection |
| **XSS from model output into the panel** | HTML-escaped, and never concatenated into executable JS: handlers are delegated from `data-*` attributes through an explicit table. CSP is `default-src 'none'` with a per-render nonce and `connect-src 'none'` |
| **Malformed code reaching disk** | Rust is parsed *before* the write, not after |
| **Silent public-API breakage** | Public surface diffed before/after; reports by default, vetoes when `api_gate_strict` is set |
| **Supply chain** | Lockfiles committed and CI builds `--locked`; the sandbox image is digest-pinned and its toolchain download is checksum-verified |
| **Runaway or hung work** | Bounded LLM connect and per-chunk stall timeouts, bounded verification subprocesses, bounded agent-loop turns |

### Not defended — by design

- **No authentication on the daemon.** It binds `127.0.0.1:50051` and trusts every local process.
  Any process running as you can dispatch a turn, and the workspace path is caller-supplied. This
  is the same posture as most local dev daemons; it is a real property, not an oversight. **Do not
  expose the port**, and do not run FreeCode on a host where you do not trust local processes.
- **No sandbox for the model's file edits.** Edits are confined to the workspace, not to a copy of
  it. Suggest mode (the default) exists so a human sees the diff first; Auto trades that for speed.
- **The `run` policy is a policy, not a jail.** The container is the actual boundary. With
  `enable_run` on and `run_in_container` off, an Allow-classified command runs on your host.
- **`style-src 'unsafe-inline'` in the panel.** The markup relies on inline styles. They cannot
  execute code, and the CSS-exfiltration vector is closed by `img-src`/`default-src`.
- **Nothing is defended against a malicious operator.** FreeCode is a tool you point at your own
  code.

## Hardening checklist

- Keep the default **Suggest** mode until you trust a given workflow.
- Leave `enable_run` off unless you need it; if you turn it on, turn on `run_in_container` too.
- Build the sandbox image before using Auto with commands.
- Set `api_gate_strict` on a release branch.
- Review `~/.freecode/global_memory.json` occasionally — the model writes to it, and it is injected
  into every workspace.
- Point the daemon at an LLM endpoint you control. It speaks plain HTTP to `127.0.0.1` by default;
  a remote endpoint means your code leaves the machine.

## Cryptography

None. FreeCode stores no credentials and performs no authentication. The one random value it
generates is the webview's per-render CSP nonce, from `crypto.randomBytes`.
