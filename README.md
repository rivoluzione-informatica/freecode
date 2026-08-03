# FreeCode

A **local-first coding agent**: a Rust gRPC daemon plus a VS Code extension, driving a model that
runs on your machine. Nothing leaves the host — no cloud API, no telemetry, no CDN.

The premise: **the model never decides — the gates do.** Every edit a model proposes passes a chain
of deterministic checks (safety scan, syntax pre-check, compile, tests, regression) before it is
allowed to touch disk. A small local model plus strict gates beats a large model plus trust.

```
VS Code extension  ──gRPC──▶  freecode-daemon  ──HTTP──▶  local LLM endpoint
   (webview UI)              (agent loop + gates)         (LM Studio, llama.cpp, …)
```

## Layout

| Path | What |
|---|---|
| `daemon/` | The brain: agent loop, tool calling, verification gates, LLM I/O |
| `cli/` | `freecode-cli` — thin gRPC client (also the harness used to drive the daemon as a model would) |
| `crates/freecode-classify/` | Deterministic task classifier (the router's input) |
| `crates/freecode-compress/` | Context compression, Tier 0 — no model, no network |
| `crates/freecode-verdict/` | The verdict spine: typed firewall, cheap-then-expensive, best-of-N |
| `crates/freecode-trajectory/` | Transcript importer → SFT / edit-pair datasets |
| `vscode-plugin/` | The VS Code extension (TypeScript) |
| `proto/freecode.proto` | The gRPC contract between the two halves |
| `docs/rfc-00*.md` | Design RFCs — the decisions and their rationale |

## Quick start

Requires Rust (stable), Node 20+, **`protoc`** (the daemon and CLI build the gRPC contract from
`proto/freecode.proto` at compile time), and a local OpenAI-compatible LLM server listening on
`127.0.0.1:1234` (LM Studio, llama.cpp, Ollama's compat endpoint, …).

```bash
brew install protobuf                 # macOS
sudo apt-get install -y protobuf-compiler   # Debian/Ubuntu
```

```bash
cargo build --release -p freecode-daemon
./target/release/freecode-daemon &            # binds 127.0.0.1:50051, loopback only

cd vscode-plugin && npm ci && node esbuild.js
```

Then open the **FreeCode** panel in VS Code and run `Developer: Reload Window`.

From the terminal instead:

```bash
freecode-cli ping
freecode-cli ask "add a doc comment to parse_config" --mode hitl --workspace "$(pwd)"
```

Progress and diagnostics go to stderr; the model's answer goes to stdout, so
`freecode-cli ask … > answer.md` gives you a clean file.

## Modes

| Mode | Behaviour |
|---|---|
| `chat` | Discussion only. No file changes, read-only tools. |
| `hitl` | **Default.** Each change is staged as a proposal you Accept or Discard. |
| `auto` | Changes are applied directly — the gates are the only safety net. |

## Configuration

Per-workspace settings live in `.freecode/config.json` (git-ignored, never committed). The gate
toggles there are how the ablation bench measures each gate's contribution.

There is no `.env` file and no dotenv loading — the few knobs are environment variables read
directly:

| Variable | Effect |
|---|---|
| `FREECODE_TIMING` | Set to anything to print per-phase timings |
| `FREECODE_ROUTE_LOG` | Path for the router telemetry JSONL (default `$HOME/.freecode/…`) |
| `FREECODE_T1_ENDPOINT` | Endpoint for the optional T1 fast-path model |
| `FREECODE_T1_MODEL` | Model id for the T1 fast-path |

Gate toggles in `.freecode/config.json` (all default ON except where noted):

| Key | Effect |
|---|---|
| `api_gate` | Diff the public Rust API before/after an edit; report removals, demotions and signature changes |
| `api_gate_strict` | **Default OFF.** Escalate `api_gate` from a warning to a hard veto — for a release branch |
| `regression_gate` | Nothing that compiled before this turn may fail after it |
| `test_gate` | **Default OFF.** Run the affected project's tests after a clean compile |
| `safety_gate` · `identity_gate` · `tiered_permissions` | Content scan, self-identification filter, blast-radius tiers |

`api_gate` reports rather than blocks on purpose: narrowing an API is frequently the actual
request, and a gate that blocks legitimate refactors gets switched off — a gate that is off is
worth nothing.

## Security posture

- **Loopback only.** The daemon binds `127.0.0.1:50051`. It has no authentication, so it trusts
  every local process — treat it like any other local dev daemon and do not expose the port.
- **Writes are confined** to the workspace: `..` and absolute paths are rejected, and the resolved
  path must land under the real workspace root (symlinks included).
- **Commands are default-deny.** A command runs unattended only if it is provably read-only or a
  test; catastrophic and exfil patterns are refused outright; everything else needs a human.
  In `auto` mode commands run inside an ephemeral `--network none` container.
- **Nothing unparseable reaches disk.** For Rust, the edited content is parsed *before* the write,
  not after — a malformed edit is refused with typed feedback instead of being written and then
  discovered by the compiler.
- **The public API is diffed, not just compiled.** `cargo check` compiles one unit, so dropping a
  `pub` from an item nothing local uses is valid Rust and ships green — the breakage lands on
  downstream crates that don't exist at check time. The API gate is the only one holding the
  "before", so it is the only one that can see what disappeared.
- **Model output is untrusted input.** It is HTML-escaped before it reaches the webview, and no
  model-supplied value is ever concatenated into executable JS: handlers are delegated from
  `data-*` attributes through an explicit action table, so markup can only reach functions on
  that table. The panel's CSP is `default-src 'none'` with a per-render nonce on the single
  `<script>` and `connect-src 'none'` — no remote script, image, font, or network call.
  (`style-src` still allows inline styles: the markup relies on them, they cannot execute code,
  and the CSS-exfiltration vector is closed by `img-src`/`default-src`.)

## Development

See [README_DEV.md](README_DEV.md) for the dev loop, the launchd service, and the gotchas.

```bash
cargo test --workspace                     # 119 tests
cargo clippy --workspace --all-targets -- -D warnings
cd vscode-plugin && npm run typecheck && npm test && node esbuild.js   # 40 tests
```

## License

MIT — see [LICENSE](LICENSE).
