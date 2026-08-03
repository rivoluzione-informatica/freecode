# Configuration

Every key here was read out of the source, not remembered. If a default below disagrees with
`GateConfig::default()` in `daemon/src/core.rs`, the code is right and this file is a bug.

There are three places settings come from, and the split is a security boundary, not filing:

| Where | Scope | Trust |
|---|---|---|
| `<workspace>/.freecode/config.json` | This project | Comes from the repo you opened — possibly untrusted |
| `~/.freecode/config.json` | You, everywhere | Yours |
| Environment variables | This process | Yours |

**Analyzers are read only from the global file, never from the project file.** A checked-out
repository must not be able to register a command the daemon will execute.

---

## Project settings — `<workspace>/.freecode/config.json`

Git-ignored by default. Nothing here is required; every key falls back to the default.

### Gates

Each gate switches off independently. That is deliberate: it is how each one's contribution is
measured, not a convenience for silencing them.

| Key | Default | Effect |
|---|---|---|
| `auto_verify` | `true` | Run compile/test verification after a write. Off = edits land unverified |
| `safety_gate` | `true` | Scan written content for secrets, merge markers, hidden/bidi characters. **Error findings block the write** |
| `identity_gate` | `true` | Filter the model claiming a forbidden identity |
| `tiered_permissions` | `true` | In Auto mode, refuse writes to full-access paths: dotfiles, `.github/workflows/`, dependency manifests, lockfiles, Dockerfiles, shell scripts |
| `regression_gate` | `true` | Fail the turn if a project that compiled before it does not compile after |
| `api_gate` | `true` | Diff the public Rust API before/after an edit and report removals, visibility demotions and signature changes. **Reports, does not block** |
| `api_gate_strict` | `false` | Promote `api_gate` from a warning to a hard veto. For a release branch |
| `test_gate` | `false` | After a clean compile, run the affected project's tests; a failure is a hard gate |
| `analyzers_gate` | `false` | Make an `error` finding from an external analyzer fail the turn instead of being report-only |

Why `api_gate` reports instead of blocking: narrowing an API is frequently the actual request. A
gate that blocks legitimate refactors gets switched off, and a gate that is off is worth nothing.
Set `api_gate_strict` when an unintended public-API change is a defect rather than a note.

### Behaviour

| Key | Default | Effect |
|---|---|---|
| `tool_calling` | `true` | Use native structured tool-calling and a bounded agent loop. Off falls back to the `<WRITE_FILE>` tag protocol (kept for ablation) |
| `compression` | `true` | Deterministic context compression at the read-file, compile-error and JSON seams, instead of blind truncation |
| `escalation_telemetry` | `true` | Append one routing record per turn (see `FREECODE_ROUTE_LOG`) |
| `excluded_files` | `[]` | Glob patterns excluded from the workspace scan, e.g. `["*.log", "dist/*", "*node_modules*"]`. Shown in the scope bar |

### The `run` tool

Off by default. The model cannot execute anything until you turn it on.

| Key | Default | Effect |
|---|---|---|
| `enable_run` | `false` | Register the `run` tool at all |
| `run_in_container` | `false` | Execute inside an ephemeral `--network none` Docker container |
| `run_container_image` | `freecode-sandbox` | Image to use. Build with `docker build -t freecode-sandbox -f docker/freecode-sandbox.Dockerfile .` |

**In Auto mode, commands run only when `run_in_container` is true.** There is no auto-execution on
the host: without the container boundary Auto refuses and tells you to use Suggest. Commands are
classified Allow / Approve / Deny by a deterministic policy before any of this — Deny is
unconditional, and Approve requires a human, which Auto cannot provide.

### T1 fast-path (experimental)

A small local model proposes a SEARCH→REPLACE edit for a trivial change; the same real gates
validate it. Fires only in Auto mode, only with an IDE selection, only on a trivially-classified
turn. On a miss or a veto the turn escalates to the main model.

| Key | Default | Effect |
|---|---|---|
| `t1_enabled` | `false` | Enable the fast path |
| `t1_endpoint` | `http://127.0.0.1:7999/v1/completions` | Raw completions endpoint (not chat) |
| `t1_model` | — | Model id at that endpoint |

These are **config keys, not environment variables**. The `FREECODE_T1_*` variables that appear in
`daemon/src/t1.rs` belong to its ignored benchmark tests and have no effect on the daemon.

### Example

```json
{
  "api_gate_strict": true,
  "test_gate": true,
  "enable_run": true,
  "run_in_container": true,
  "excluded_files": ["*.log", "dist/*", "*node_modules*"]
}
```

---

## Global settings — `~/.freecode/config.json`

### Analyzers

External tools that receive the changed files (or the diff) and return findings as JSON. This is
how you bolt on clippy, eslint, semgrep or bandit without touching the daemon.

Read **only** from this file. Report-only unless `analyzers_gate` is on; they never edit.

```json
{
  "analyzers": [
    {
      "name": "clippy",
      "command": ["cargo", "clippy", "--message-format", "short"],
      "input": "none",
      "extensions": ["rs"],
      "timeout_secs": 60
    }
  ]
}
```

| Field | Default | Meaning |
|---|---|---|
| `name` | required | Shown in the verdict |
| `command` | required | argv. Not a shell string — no shell is involved |
| `input` | `"files"` | `"files"` appends the changed paths as arguments; `"diff"` pipes `git diff` to stdin; `"none"` passes nothing |
| `extensions` | `[]` (any) | Only run when a changed file matches |
| `timeout_secs` | `30` | Killed past this |

Expected on stdout:

```json
[{"severity":"error|warn|info","file":"src/x.rs","line":12,"message":"…","rule":"…"}]
```

### Cross-project memory

`~/.freecode/global_memory.json` holds notes injected into every workspace's context. **It is
written by the model**, not only by you — the write-scope bar in the panel exists to make that
visible. Edit it from the Memory panel.

---

## Environment variables

| Variable | Default | Effect |
|---|---|---|
| `FREECODE_ROUTE_LOG` | platform-dependent, see below | Where routing telemetry is appended, one JSON line per turn |
| `FREECODE_TIMING` | unset | Set to anything to print per-phase timings to the daemon's stdout |

The route-log default follows the platform, in this order (`escalation.rs::route_log_path`):

1. `$FREECODE_ROUTE_LOG` — wins everywhere, on every platform
2. macOS: `~/Library/Logs/freecode-route.jsonl`
3. elsewhere: `$XDG_STATE_HOME/freecode/route.jsonl`, falling back to `~/.local/state/freecode/route.jsonl`
4. no home at all (a container without `$HOME`): `/tmp/freecode-route.jsonl`

State, not cache and not config: machine-local history that should survive a reboot and means
nothing on another machine.

---

## Extension settings

In the panel's **Settings**, stored in the webview and sent with each dispatch:

| Setting | Default | Effect |
|---|---|---|
| LLM Endpoint URL | `http://127.0.0.1:1234/v1/chat/completions` | Overrides the daemon default for this session |
| LLM Model Name | `gemma-4-e2b-it-mlx` | Model id to request |
| Excluded Files | — | Writes `excluded_files` into the project config |
| Monotonicity Curation | off | Export only successful trajectories |

---

## CLI

```
freecode-cli doctor [--workspace <path>] [--endpoint <url>] [--json]
freecode-cli ping
freecode-cli ask <prompt> [--mode chat|hitl|auto] [--workspace <path>]
                          [--endpoint <url>] [--model <id>] [--session <id>]
```

`ping` reports the daemon's real crate version (it used to return a hardcoded string that stayed
`0.1.0` across four releases). The extension compares that against its own and warns on a mismatch.

`doctor` exits `1` when something required is missing, `0` otherwise — usable in a script.
`--json` emits one object with a `checks` array plus `missing` and `warnings` counts.

For `ask`, the model's answer goes to **stdout** and every progress line, gate verdict and metric
goes to **stderr**, so `freecode-cli ask … > answer.md` gives you a clean file.
