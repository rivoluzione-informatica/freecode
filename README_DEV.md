# freecode — internal dev guide

> Local-LLM coding agent: a Rust gRPC **daemon** (`127.0.0.1:50051`) + a **VSCode extension**.
> Brand "FreeCode", domain freecoders.org. This file = how to run it, change it, and not trip on the
> dev-loop gotchas. (Architecture/decisions live in `docs/rfc-00*.md`.)

---

## 1. Layout

```
daemon/                  Rust gRPC daemon (the brain): agent loop, gates, LLM calls   → bin freecode-daemon
cli/                     thin gRPC client (freecode-cli)
crates/
  freecode-classify/     deterministic RFC-004 task classifier (shared)
  freecode-compress/     RFC-003 context compression (Tier 0)
  freecode-verdict/      the verdict spine: typed firewall + cheap-then-expensive + best-of-N
  freecode-trajectory/   RFC-005 importer: Claude Code transcripts → SFT / Battle-1 datasets
vscode-plugin/           the VSCode extension (TS); webview = src/webview/{markup,client,style}.ts
proto/freecode.proto     the gRPC contract (daemon ⇄ plugin)
docs/rfc-00*.md          design RFCs (001 tool-calling · 002 run · 003 compress · 004 ladder · 006 tiered)
```

The daemon talks to a local OpenAI-compatible LLM endpoint (default LM Studio
`http://127.0.0.1:1234/v1/chat/completions`). The plugin only **connects** to the daemon — it never
spawns it.

## 2. Prerequisites
- Rust (stable), Node + npm, a running local LLM (e.g. LM Studio) on `:1234`.
- **`protoc`** — `tonic-build` invokes it from `daemon/build.rs` and `cli/build.rs`; without it the
  build dies with `Could not find protoc`. `brew install protobuf` / `apt-get install protobuf-compiler`.
- macOS (the launchd service + the dev symlink below assume macOS paths).

## 3. Quick start (first time)
```bash
cargo build --release -p freecode-daemon      # build the daemon
./target/release/freecode-daemon &            # or install the LaunchAgent (§7) and use `make daemon`
make plugin                                    # build the extension bundle
```
Then in VSCode: open the **FreeCode** panel and run **`Developer: Reload Window`**. The green
"Online" badge = the panel reached the daemon on `:50051`.

## 4. The dev loop

### Changed the **daemon** (Rust)?
```bash
make daemon        # = cargo build --release -p freecode-daemon  +  launchctl kickstart (restart)
```
**If you also run a hand-started daemon:** the LaunchAgent has `KeepAlive=true`, so launchd respawns
its own instance within ~1s of any kill — a manual `./target/release/freecode-daemon` will then die with
`Address already in use`. Verify which process actually holds the port before trusting a test:

```bash
lsof -nP -iTCP:50051 -sTCP:LISTEN     # the PID that is really serving
```

To restart by hand instead of via launchd:
```bash
cargo build --release -p freecode-daemon
kill "$(lsof -tiTCP:50051 -sTCP:LISTEN)" 2>/dev/null   # stop the old one
./target/release/freecode-daemon &                     # start the fresh one
lsof -nP -iTCP:50051 -sTCP:LISTEN                       # verify it bound
```
A daemon change is NOT live in the panel until the daemon is restarted (the binary is reloaded, not hot).

### Changed the **plugin** (TS / webview)?
```bash
make plugin        # typecheck (tsc) + bundle (esbuild) + copy proto + a webview-JS syntax check
```
Then **`Developer: Reload Window`** in VSCode. No daemon restart needed (the daemon already emits the events).

### ⚠️ The one that bites: the installed-extension symlink
VSCode runs the **INSTALLED** extension at `~/.vscode/extensions/freecode.freecode-vscode-0.1.0/`,
**not** `vscode-plugin/dist`. We symlinked its `dist` → the dev `dist` so `make plugin` + reload applies
instantly:
```bash
ls -la ~/.vscode/extensions/freecode.freecode-vscode-0.1.0/dist   # should be a symlink → vscode-plugin/dist
# if it's a real dir (after a reinstall), re-link it:
mv ~/.vscode/extensions/freecode.freecode-vscode-0.1.0/dist{,.orig}
ln -s "$PWD/vscode-plugin/dist" ~/.vscode/extensions/freecode.freecode-vscode-0.1.0/dist
```
Without this symlink, **plugin changes silently never reach the running panel** (you'll edit, build,
reload, and see the old UI). `vscode-plugin/dist` is gitignored (build artifact).

## 5. Build & test
```bash
cargo test --workspace        # or: make test    — all crates
cargo build --workspace       # compile everything
make plugin                   # bundle the extension (typecheck + esbuild)
```
Every Rust change must keep `cargo test --workspace` green. Webview JS is syntax-checked by `make plugin`
(it `new Function(getWebviewJs())`s the bundle).

## 6. Config — the gates
Two files (deterministic gates are **per-repo**, the security-sensitive `run` switch is **global-only**):
- `<workspace>/.freecode/config.json` — gate toggles (all default ON except where noted):
  `auto_verify`, `safety_gate`, `identity_gate`, `tiered_permissions`, `regression_gate`,
  `tool_calling`, `compression`, `escalation_telemetry`; **`test_gate`** (default **OFF** — runs the
  affected project's tests after compile; heavier), `analyzers_gate` (default OFF).
- `~/.freecode/config.json` (GLOBAL only, so a cloned repo can't enable shell exec):
  `enable_run` (default **false**), `run_in_container`, `image`. The `run` tool is OFF by default.

Modes (in the panel): **SUGGEST** (=hitl, stages proposals you Accept/Discard) · **AUTO** (writes
through the gates) · **CHAT** (read-only).

## 7. Gotchas / current state
- **launchd works — and it fights you.** (This entry used to say launchd was WEDGED; verified
  2026-08-03 that it is not.) The agent has `KeepAlive=true`, so it respawns the daemon within ~1s of
  any `kill`/`pkill`. Consequences when testing: (a) your hand-started daemon dies with `AddrInUse` and
  the launchd one keeps serving; (b) its stdout goes to `~/Library/Logs/freecode-daemon.log`, **not** to
  your redirect — look there for `[safety]`/`[llm-retry]` lines. Always confirm the serving PID and its
  binary mtime before concluding anything from a test:
  ```bash
  PID=$(lsof -nP -iTCP:50051 -sTCP:LISTEN -Fp | tr -d p); ps -o lstart=,args= -p "$PID"
  stat -f "%Sm" -t "%H:%M:%S" target/release/freecode-daemon   # must predate the process start
  ```
- **Webview JS is one big template literal** (`getWebviewJs()` returns a backtick string): inside it,
  NO backticks, NO `${`, and **regex backslashes must be doubled** (`/turn (\\d+)/`) or the literal eats
  them → syntax error. `make plugin` catches it loudly.
- **No inline event handlers.** The CSP is `script-src 'nonce-…'`, so an `onclick="…"` attribute
  silently does nothing. Add `data-action="myFn"` (or `data-change`/`data-input`/`data-mousedown`/
  `data-keydown`), put arguments in `data-a1`/`data-a2`, and register the function in `FC_ACTIONS`
  at the bottom of `client.ts`. `npm test` fails the build if a handler creeps back in or an action
  has no table entry.
- **Webview tests**: `cd vscode-plugin && npm test` (node:test, zero extra deps). The suite bundles
  `client.ts`, evaluates the returned string under DOM stubs (`test/harness.js`) and exercises
  `escapeHtml`/`parseMarkdown`/`fcDispatch` directly — that is the only way to reach them, since
  they live inside the template literal.
- **API Surface Gate** (`daemon/src/api_surface.rs`): the only gate with a "before". Parses the
  Rust file pre- and post-edit with `syn` and diffs the externally reachable surface. Warn-class
  by default (`api_gate`); `api_gate_strict` makes it a veto. If you extend it, the tests that
  matter are the NEGATIVE ones — additions, private items, reformatting and param renames must
  stay silent, or nobody keeps the gate on.
- **Route telemetry** (RFC-004): each turn appends a line to `~/Library/Logs/freecode-route.jsonl`
  (`$FREECODE_ROUTE_LOG` to override) — `would_route ∈ {ship, retry-same-tier, escalate-to-T2}`. Drives
  nothing yet; it measures the escalate band.
- **Datasets** (RFC-005): regenerate from your local Claude Code corpus. The corpus is yours and
  stays local — never push the generated files. Build the directory allowlist DELIBERATELY: pass
  only the projects you actually intend to mine, and exclude anything you are not free to.
  ```bash
  ls -d ~/.claude/projects/*/ | grep -vE '<projects-to-exclude>' > /tmp/safe_dirs.txt
  cargo run -q -p freecode-trajectory --example export -- /tmp/fc_sft_full.jsonl $(cat /tmp/safe_dirs.txt)  # SFT
  cargo run -q -p freecode-trajectory --example edits  -- /tmp/fc_battle1.jsonl  $(cat /tmp/safe_dirs.txt)  # Battle-1 AST pairs
  ```
- **Remote**: `origin` = github.com/rivoluzione-informatica/freecode. Commit/push only when intended.

## 8. Where to read next
- `docs/rfc-006-tiered-generate-and-validate.md` — the architecture (tiers, validator stack, the verdict
  firewall, the AIMP decision). The others: rfc-001 tool-calling · rfc-002 `run` · rfc-003 compression ·
  rfc-004 escalation ladder.
- `crates/freecode-verdict/src/lib.rs` — the verdict spine (firewall + COMPILOT-validated mechanics).
