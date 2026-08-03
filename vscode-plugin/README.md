# FreeCode

A coding agent that runs entirely on your machine, and that you can check.

The model proposes; **deterministic gates decide**. Every edit passes a chain of checks — secrets,
syntax, public API, compile, tests, regression — before it is allowed to touch disk. Nothing leaves
your computer: no cloud API, no telemetry, no CDN.

---

## What you need first

This extension is a client. It talks to the **FreeCode daemon** on `127.0.0.1:50051`, which in turn
talks to **your own LLM server** on `127.0.0.1:1234`.

```
this extension ──gRPC──▶ freecode-daemon ──HTTP──▶ your local LLM
```

1. **A local OpenAI-compatible LLM server** — [LM Studio](https://lmstudio.ai), `llama.cpp --server`,
   or Ollama's compatibility endpoint. Load a model and note its id.
2. **The daemon** — download the archive for your platform from
   [Releases](https://github.com/rivoluzione-informatica/freecode/releases), or build from source.

```bash
tar xzf freecode-<your-platform>.tar.gz && cd freecode-<your-platform>
./freecode-cli doctor      # tells you exactly what is missing, and the command that fixes it
./freecode-daemon &        # binds 127.0.0.1 only
```

`doctor` is the fastest way to find out why something is not working. It checks the toolchain,
`protoc`, Node, the workspace, the daemon and the LLM endpoint, and prints the fix for each
problem. It exits non-zero when something required is missing, so it works in scripts too.

Open the **FreeCode** panel in the activity bar. The status dot turns green when it reaches the
daemon; click it to re-check.

---

## The three modes

The mode belongs to the message you are about to send. Pick it next to the send button.

| Mode | What happens |
|---|---|
| **Suggest** | *Default.* Each change is staged as a proposal with a diff. Nothing is written until you Accept. |
| **Auto** | Changes are applied directly. The gates are the only thing between the model and your files. |
| **Chat** | Discussion only. The model can read files but can never write. |

Auto is tinted amber in the picker, because it writes without asking and you should be able to see
that without reading the word.

Greetings and read-only questions are detected and short-circuited even outside Chat — asking
"what does this function do" does not start a verification pipeline.

---

## The write scope bar

Above the input, one line states **everywhere FreeCode can write**. Click to expand.

It is not decoration. Three of the locations are *outside* your repository — including
`~/.freecode/global_memory.json`, which holds notes written by the model — and the bar exists so
you know that without reading the source. Green means inside the open repository, amber means
outside. A row lights up when that location is actually written during a turn.

---

## The gates

Each turn streams a verdict per gate into the panel. Green is not decoration either — it is the
record of what was checked.

| Gate | Refuses when |
|---|---|
| **Injection** | The prompt, an ingested memory, or the workspace's own system prompt carries a prompt-injection pattern |
| **Permission** | In Auto mode, the write targets a full-access path (dotfiles, CI, manifests, scripts) |
| **Slop & Safety** | The content carries a secret, a merge marker, or hidden/bidi characters |
| **Syntax** | The edited Rust does not parse — checked *before* the write, so malformed code never reaches disk |
| **API Surface** | A public item was removed, demoted, or had its signature changed (reports by default) |
| **Compiler** | `cargo check` / `tsc` / `cmake` / `compileall` fails after the edit |
| **Test** | The project's tests fail (opt-in) |
| **Regression** | Something that compiled before this turn does not compile after it |
| **Run** | The command is destructive, exfiltrating, or an escalation — refused outright |
| **Identity** | The model claims to be something it is not |

A hard veto means the turn cannot ship, no matter how confident the model is. Failures come back
to the model as typed feedback (`[reason: syntax_error]`, `[reason: api_surface]`) so it can
correct itself on the next attempt instead of repeating the same edit.

---

## The panel

| Control | What it opens |
|---|---|
| **Git** | Working-tree status; click a file to open it, or the Δ to see its diff |
| **Memory** | Project and cross-project notes injected into context. Add, edit and delete them here |
| **Harness** | Cost and confidence for the turn, plus the cumulative session total |
| **Pipeline** | The per-turn strip: intent → context → gates → agent → done. Click to cycle compact / full / hidden |
| **AST Edit** | Replace a named symbol's body directly, without going through the model |
| **Settings** | LLM endpoint, model id, and glob patterns to exclude from scans |
| **Export** | Write the session trajectory to `.freecode/trajectories/` |

In Suggest mode a proposal arrives with a diff, an **Edit Code** toggle to adjust it before
applying, and Accept / Discard for the whole group.

---

## Configuration

Per-project settings live in `.freecode/config.json`, which is git-ignored. Every gate can be
switched off individually — that is how their contribution is measured, not a convenience.

```json
{
  "api_gate_strict": true,
  "test_gate": true,
  "excluded_files": ["*.log", "dist/*"]
}
```

Full reference: [CONFIGURATION.md](https://github.com/rivoluzione-informatica/freecode/blob/main/docs/CONFIGURATION.md).

---

## When something does not work

Run `freecode-cli doctor` first — it answers most of it.

| Symptom | Cause |
|---|---|
| Status dot stays red | The daemon is not running. Start it, then click the dot |
| Turn never answers | No model loaded in your LLM server. `doctor` shows the loaded models |
| "cannot find protoc" while building | `brew install protobuf` / `apt-get install protobuf-compiler` |
| Auto mode refuses to run a command | Auto only executes inside the container boundary. Use Suggest, or build the sandbox image |
| Panel controls do nothing | Reload the window (`Developer: Reload Window`) after rebuilding the extension |

More: [TROUBLESHOOTING.md](https://github.com/rivoluzione-informatica/freecode/blob/main/docs/TROUBLESHOOTING.md).

---

## Security posture

- The daemon binds `127.0.0.1` and has **no authentication** — it trusts every local process.
  Treat it like any other local dev daemon and do not expose the port.
- Writes are confined to the workspace: `..` and absolute paths are rejected, and the resolved
  path must land under the real root — symlinks included.
- Commands are default-deny. In Auto mode they only run inside an ephemeral `--network none`
  container.
- Model output is untrusted input: HTML-escaped, and never concatenated into executable JS. The
  panel's CSP admits one nonce-carrying script and blocks every remote and network call.

MIT licensed. Source and issues:
[github.com/rivoluzione-informatica/freecode](https://github.com/rivoluzione-informatica/freecode).
