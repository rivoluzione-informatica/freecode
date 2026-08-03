# Troubleshooting

**Start here:**

```bash
freecode-cli doctor
```

It checks the toolchain, `protoc`, Node, the workspace, the daemon and the LLM endpoint, prints the
fix for anything wrong, and exits non-zero when something required is missing. Most of what follows
is a longer explanation of something `doctor` already told you in one line.

---

## Nothing happens when I send a message

**The status dot is red.** The daemon is not running. Start it and click the dot to re-check:

```bash
./freecode-daemon &          # or: cargo run --release -p freecode-daemon
```

**The dot is green but the turn never answers.** The daemon reached the LLM socket but no model is
loaded, or the model id is wrong. `doctor` lists what the server actually exposes:

```
[ok  ] llm endpoint  127.0.0.1:1234 — qwen3-8b, gemma-3-12b (+4 more)
```

Put one of those ids in **Settings → LLM Model Name**. If the list is empty, load a model in your
LLM server.

**It answers, then stops mid-sentence.** The stream stalled. The daemon aborts a turn after 120s of
silence between chunks rather than hanging forever; the panel says so. Usually the model server ran
out of memory — check its own log.

---

## "Version mismatch: extension X, daemon Y"

The extension and the daemon are separate programs that speak one gRPC contract. They ship as a
pair and are versioned in lockstep, so a mismatch means one half is from a different release.

This warning exists because the failure it prevents is a bad one: the panel looks like it works,
then a field the newer half expects is not there, and the symptom shows up far from the cause.
Before the check existed, the daemon reported its version, the panel printed it, and nothing
compared them.

```bash
cargo build --release -p freecode-daemon    # rebuild the daemon from this checkout
./target/release/freecode-cli ping          # confirm the version it now reports
```

or install the extension from the release matching your daemon.

Only MAJOR.MINOR is compared — patch releases do not change the contract. An unparseable version
(a fork, a dev build) is never reported as a mismatch: a check that cries wolf gets ignored.

## Build failures

### `failed to run custom build command` / `Could not find protoc`

`tonic-build` shells out to `protoc` to compile `proto/freecode.proto`, and it is not bundled.

```bash
brew install protobuf                        # macOS
sudo apt-get install -y protobuf-compiler    # Debian/Ubuntu
```

### `error: the lock file needs to be updated` in CI

Something changed a manifest without refreshing `Cargo.lock`. Run `cargo update -w` and commit the
lockfile — CI builds `--locked` on purpose, so a stale lock is a failure rather than a silent
resolution to different versions.

### The extension builds but the panel is blank or its buttons do nothing

The webview JS is one large template literal. A stray escape inside it produces dead JavaScript
that `tsc` cannot see, which is why `node esbuild.js` compiles it separately and fails loudly.
If the build passed and the panel is still inert:

1. Run `Developer: Reload Window` — a rebuilt bundle is not picked up until you do.
2. Open the webview devtools (`Developer: Open Webview Developer Tools`) and look for
   `FreeCode: unknown data-action: <name>`. That means markup references a handler with no entry
   in the `FC_ACTIONS` table — add it in `client.ts`.

Note that inline `onclick` attributes **do nothing**: the CSP is nonce-based and every handler is
delegated from `data-action`. `npm test` fails the build if an inline handler creeps back in.

---

## The agent edits nothing

**You are in Chat mode.** Chat can read but never writes. Switch to Suggest.

**You are in Suggest mode and expected files to change.** They do not, by design — Suggest stages a
proposal and waits for Accept. That is the default because it is the safe one.

**A gate refused.** The panel shows which and why. The common ones:

| Verdict | Meaning |
|---|---|
| `error[permission_tier]` | Auto mode refuses full-access paths — dotfiles, CI workflows, manifests, lockfiles, scripts. Switch to Suggest |
| `error[safety_gate]` | The content carries a secret, a merge marker, or hidden characters. It was not written |
| `error[syntax_error]` | The result does not parse. Caught before the write, so nothing was damaged |
| `error[api_surface]` | `api_gate_strict` is on and the edit changes the public API. Preserve the items, or set it back to `false` |
| `error[unsafe_path]` | The path escapes the workspace — `..`, absolute, or a symlink pointing outside |

---

## Commands are refused

**"blocked by FreeCode's command policy".** The command is in the unconditional Deny set:
destructive (`rm -rf`), exfiltrating (`curl … | sh`, reading `~/.ssh`), or an escalation (`sudo`).
It will not run with approval either. This is deliberate and not configurable.

**"needs explicit human approval, which isn't available in auto mode".** The command is neither
provably safe nor catastrophic, so it falls to a human. Auto has no approval UI — use Suggest.

**"auto-exec requires the container boundary".** Auto executes only inside the sandbox:

```bash
docker build -t freecode-sandbox -f docker/freecode-sandbox.Dockerfile .
```

then set `"run_in_container": true` in `.freecode/config.json`. Without Docker, Suggest mode still
runs commands after you approve them.

**Nothing runs at all.** `enable_run` defaults to `false`. The model has no shell until you grant it
one.

---

## The model keeps retrying the same thing

It received a veto it cannot satisfy — usually `api_gate_strict` when the change you asked for *is*
a public-API narrowing. The refusal message tells the model to stop and explain rather than retry,
but a small model may still loop for a turn or two. Set `"api_gate_strict": false` for that change.

---

## macOS: the daemon keeps coming back after I kill it

You installed the LaunchAgent, and it has `KeepAlive=true` — launchd respawns the daemon within a
second of any `kill`. Consequences while testing:

- A hand-started daemon dies with `Address already in use`; launchd's instance is the one serving.
- Its stdout goes to `~/Library/Logs/freecode-daemon.log`, **not** to your redirect.

Confirm which process is actually serving before concluding anything:

```bash
PID=$(lsof -nP -iTCP:50051 -sTCP:LISTEN -Fp | tr -d p)
ps -o lstart=,args= -p "$PID"
```

To stop it properly: `launchctl bootout gui/$(id -u)/org.freecoders.freecode-daemon`.

---

## Linux notes

The daemon is portable Rust and runs fine.

- Routing telemetry follows XDG here: `$XDG_STATE_HOME/freecode/route.jsonl`, falling back to
  `~/.local/state/freecode/route.jsonl`. Override with `$FREECODE_ROUTE_LOG` if you want it
  elsewhere. (It used to hardcode the macOS path on every platform, which created a `~/Library`
  directory that belongs to no convention on Linux.)
- There is no launchd. Use `scripts/freecode-daemon.service` — a systemd **user** unit, since the
  daemon runs as you and binds loopback. Install instructions are in its header; logs go to
  `journalctl --user -u freecode-daemon -f`.

---

## Getting a useful bug report together

```bash
freecode-cli doctor --json > doctor.json
```

Plus: the gate verdict that failed (copy it from the panel), the model id, and whether the same
prompt behaves differently in Suggest versus Auto. The daemon's stdout carries `[safety]`,
`[llm-retry]` and `[edit]` lines that usually pin the cause.
