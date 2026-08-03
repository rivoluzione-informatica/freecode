# RFC-002 — `run` tool (gated shell execution)

Status: **IMPLEMENTED** (Slices 0, 1, 2, 3) · Date: 2026-06-22 · Scope: `freecode-daemon` + `vscode-plugin`

> Built: §5 Slice 0 (`run_policy::classify_command`), Slice 1 (`run` tool, HITL-only, global
> `enable_run`), Slice 3 (Docker sandbox `docker/freecode-sandbox.Dockerfile` + `run_in_container`
> + **container-gated auto-exec**: `auto` runs `Allow` ONLY inside the no-network ephemeral container,
> never on the host), and **Slice 2 (2026-06-22): HITL command approval** — an `Approve`-class command
> in HITL is STAGED as a proposal (`status:proposal`, `kind:run-command`); on Accept the extension
> re-dispatches it via the new `approved_command` request field, and the daemon short-circuits (no LLM)
> and **re-validates** (`gate_approved_command`: requires global `enable_run` + re-classifies, so a
> tampered/stale/replayed approval can never run a `Deny`). AUTO/chat stay fail-closed. Default OFF.
> Live activation needs a daemon rebuild+restart + a VSCode window reload.

## 1. Goal & tension

Give the agent a `run` tool (execute shell commands: tests, linters, builds) **without
turning freecode into an arbitrary-RCE foot-gun**. `run` is the single highest-risk
capability: command + the model (or a prompt injection ingested from a repo file/memory)
= remote code execution. pi punts on this (no permission system; relies on the operator
containerizing — Gondolin micro-VM / Docker). freecode's whole differentiator is
**deterministic gates**, so `run` must be **gated by default**, not free.

Non-negotiable: `run` must NOT weaken the existing safety posture (path-traversal guard,
auto-build RCE gate, injection gate, tiered permissions). It is OFF unless explicitly
enabled, and even then default-deny.

## 2. Threat model

- Model hallucination or prompt-injection (from an ingested README/memory/file) emits a
  destructive or exfiltrating command (`rm -rf`, `curl … | sh`, `git push`, package
  installs that run arbitrary postinstall scripts, network egress of secrets).
- Untrusted repo: a checked-out project must not be able to auto-enable or auto-approve
  `run` (same class as the analyzer-config RCE we already closed by reading analyzer
  config from the GLOBAL config only).

## 3. Design (proposed)

### 3.1 Enable switch (global only)
- `run` is registered as a tool ONLY if `enable_run: true` in the **global**
  `~/.freecode/config.json` (NOT the per-repo `.freecode/config.json`) — so a cloned repo
  can't turn it on. Default **false**.

### 3.2 Deterministic command policy — `classify_command(cmd) -> Verdict`
Three-way, default-deny:
- **Allow** (run without approval): a small, configurable allowlist of read-only / test
  commands, matched on the *program* + safe subcommands, e.g. `cargo test|check|clippy`,
  `npm test`, `pnpm test`, `pytest`, `go test`, `ls`, `cat`, `grep`/`rg`, `git status|diff|log|show`.
- **Deny** (hard block, never run): destructive / exfil / escalation patterns —
  `rm -rf`, `sudo`, `chmod 777`, `:(){…}`, pipes to a shell (`| sh`, `| bash`),
  `curl`/`wget` with network, `git push`, `npm/pip/cargo install|add`, redirections that
  escape the workspace, env-dumping (`env`, `printenv`, reading `.env`/`~/.ssh`).
- **Approve** (everything else): requires explicit human approval before running.

Implemented like `safety_gate::classify_tier` — pure, deterministic, unit-tested,
tokenized (not naive substring), with a `Verdict` enum.

### 3.3 Mode behavior
- **chat**: `run` never executes (parity with no-write).
- **hitl**: `Allow` runs; `Approve` → emit a **command proposal** (reuse the proposal/
  Accept flow — a `proposal` variant carrying the command) → run only on Accept; `Deny`
  → refused tool_result.
- **auto**: `Allow` runs; `Approve` → **refused** (fail-closed — no UI to approve) with a
  tool_result telling the model to switch to HITL; `Deny` → refused.

### 3.4 Execution
- `tokio::process` (like analyzers): timeout (config, default 60s) + `kill_on_drop` +
  stdout/stderr truncation (e.g. 4 KB / 200 lines) + `cwd = workspace`. Result (exit code +
  truncated output) folded into the `tool_result` so the model can react. Emit
  `tool_call`/`tool_result` + a `Run Gate` verdict `{level, reasons[]}`.
- Optional `run_env_clean: true` to strip the environment (no inherited secrets).

### 3.5 Containerization (the real boundary; defense-in-depth)
- Ship a `Dockerfile` + a doc: run the whole daemon (or just `run`) inside a container
  with the workspace mounted, no host network. Optional `run_in_container` config that
  routes commands via `docker exec`. Policy + approval are belt; the container is the
  boundary (à la pi's Gondolin).

## 4. Why this is safe-by-default
OFF unless globally enabled → default-deny policy → Approve needs a human (fail-closed in
auto) → Deny is unconditional → timeouts/truncation/cwd-confinement → optional clean-env
and container. The injection gate + secret-scan already run on ingested context, so a
command synthesized from untrusted input still can't auto-run without crossing the policy.

## 5. Slices
- **Slice 0**: `classify_command` (+ enum, allow/deny lists, unit tests). No tool yet.
- **Slice 1**: `run` tool behind global `enable_run`, Allow/Deny only; Approve auto-refused
  (fail-closed). Folded into the agent loop + Run Gate verdict. (Auto-safe.)
- **Slice 2**: HITL command approval (command proposal → Accept → execute). **DONE 2026-06-22.**
- **Slice 3**: container recipe + optional `run_in_container`.

## 6. Open questions (for Fab)
1. **Do we even want `run`?** Given the "nabba" reservation about agentic autonomy, a
   verified gate-first agent might deliberately *not* auto-run shell. Maybe `run` stays
   HITL-only forever (no auto), as a principled stance.
2. Default **allowlist** contents — start with the test/check commands above?
3. **Container now or later** — ship the Docker recipe in Slice 1 so `auto` users have a
   real boundary before any `Approve`-class execution exists?
4. Allowlist/denylist in **global config** (editable) vs **compiled-in** (tamper-proof)?

## 7. Decisions (2026-06-20)

Recommendations to unblock Slice 0; **Q1 still needs Fab** (tied to the "nabba"
thesis on agentic autonomy):

- **Q1 — do we even want `run`?** LEAN: ship `run` **HITL-only first** (NO `auto`
  execution at all). A human approves every command; `auto`-exec is a separate, later
  decision. This is the principled gate-first stance and matches the nabba skepticism —
  freecode would deliberately *not* auto-run shell. Concretely: **defer §5 Slice 1**
  (auto Allow/Deny) and start with **§5 Slice 2** (HITL command approval); the `auto`
  Allow/Deny exec path is a separate, later decision. **(Awaiting Fab's call.)**
- **Q2 — default allowlist:** read-only / test commands only —
  `cargo test|check|clippy`, `npm|pnpm|yarn test`, `pytest`, `go test`, `ls`, `cat`,
  `rg`/`grep`, `git status|diff|log|show`.
- **Q3 — container:** **later (Slice 3)**. Slice 1 ships policy + HITL approval +
  fail-closed; the Docker recipe must land *before* any `auto` exec is ever enabled.
- **Q4 — allow/deny location:** **compiled-in defaults** (tamper-proof) + optional
  **global-config** extension only (never per-repo).

Net: **§5 Slice 0** (`classify_command` + unit tests — no execution surface) can proceed
regardless of the Q1 call. The HITL-only lean de-risks the *executing* slices: it defers
§5 Slice 1's `auto` exec, so no arbitrary `auto` exec exists until a container boundary
does.

See [[rfc001-progress-20260620]] for the tool-loop this plugs into, and
[[freecode-pi-reference]] (Gondolin/containerization) for the isolation patterns.
