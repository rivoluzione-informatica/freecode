# RFC-001 — Structured tool-calling (multi-turn agent loop)

Status: **IMPLEMENTED** — Slices 0–3 + read_file/edit landed; `tool_calling` DEFAULT ON (commit 3561bf0) · Date: 2026-06-19 (updated 2026-06-20) · Scope: `freecode-daemon` (+ minor webview/CLI)

## 1. Goal

Replace freecode's `<WRITE_FILE>` text-tag protocol with **native OpenAI structured
tool-calling**, driven by a **bounded multi-turn agent loop**, while keeping the tag
protocol as a fallback. The deterministic gate pipeline becomes the *interceptor*
between argument-validation and tool execution — freecode's verification moat applied
to pi-style plumbing.

Settled decisions (2026-06-19):
- **First slice exposes one tool: `write_file`** (like-for-like with today). Later: `read_file` → `edit` → `run`.
- **Correct multi-turn loop from the start** (model → tool_calls → execute+gate → tool_result re-injected → continue), minimal toolset, iteration-capped.

## 2. Empirical basis (verified, not assumed)

Probed `127.0.0.1:1234/v1/chat/completions` with a real `tools` definition. All local
models tested returned valid `tool_calls` with schema-correct arguments:

| model | result |
|---|---|
| gemma-4-e2b-it-mlx (default) | ✅ valid `write_file(path, content)` |
| qwen/qwen3-4b-2507 | ✅ valid |
| gemma-3-12b-it-qat | ✅ valid |
| qwen3-0.6b | ✅ valid |

→ Tool-calling is viable as the **primary** path locally. The tag fallback exists for:
(a) endpoints/models that reject `tools`; (b) reliability on multi-tool / multi-call
turns; (c) malformed-args recovery.

## 3. Current state (what changes)

- `llm.rs`: `ChatCompletionRequest { model, messages, temperature, stream }`;
  `ChatMessage { role, content }`; streamed delta only carries `choices[].delta.content`.
- `core.rs::dispatch_intent`: a **retry loop** (`max_retries=3`) that sends ONE streaming
  response, regex-parses `<WRITE_FILE>`/`<LEARN>` from the text, runs gates + writes +
  compile, then retries (push a user message) or finalizes. It is single-response; the
  "loop" is for self-correction, not tool calls.
- Reason-coded retry, gates, HITL staging, telemetry as described in
  `[[checkpoint-20260619]]`.

## 4. Proposed design

### 4.1 LLM layer (`llm.rs`)
- Extend `ChatMessage` to the OpenAI shape: optional `tool_calls` (on assistant
  messages) and `tool_call_id` (on `role:"tool"` messages). Keep `content` optional.
- Extend `ChatCompletionRequest` with `tools: Option<Vec<ToolDef>>` and
  `tool_choice: Option<String>` (`"auto"`).
- Extend streaming-delta parsing to accumulate `tool_calls` across deltas
  (each delta carries `index`, partial `id`, `function.name`, `function.arguments`
  fragments; assemble per `index`, then `serde_json::from_str` the assembled arguments).

### 4.2 Tool registry
- A `Tool` = { name, JSON-schema params, `execute(args, ctx) -> ToolResult` }.
- Slice 1 registers exactly one: `write_file { path: string, content: string }`.
- Structured so adding `read_file`/`edit`/`run` later is registration-only.

### 4.3 The bounded agent loop (replaces the single-response retry loop)
```
iters = 0
loop {
    resp = stream LLM(messages, tools)            // tokens stream to UI as today
    append assistant message (content + tool_calls) to messages
    if resp has no tool_calls:                     // model is done
        finalize (emit final status); break
    for tc in resp.tool_calls:
        args = validate_against_schema(tc)         // malformed -> tool_result error
        verdict = run_gates(tc, args)              // INTERCEPTOR: injection/identity already
                                                   //   pre-flight; permission + slop&safety here
        if verdict.blocked:
            tool_result = error(reason-coded)      // model sees it, can fix -> next iter
        else:
            out = tool.execute(args)               // write_file (or stage, in HITL)
            tool_result = out + post-checks        // compile/regression result folded in
        append tool_result (role:"tool", tool_call_id) to messages
    iters += 1
    if iters >= max_tool_iters: finalize-with-cap-notice; break
}
```
- **Retry unification**: a compile error or a gate rejection is just the `tool_result`
  the model reacts to on the next iteration. The separate reason-coded retry machinery
  collapses into tool-result feedback (same reason codes, carried in the result text).
- **Cap**: `max_tool_iters` (default ~6) prevents runaway. Replaces `R` on the tool path.
- **Compiler/Regression gates**: run after a `write_file` executes; their verdict is
  folded into that tool's result so the model sees "wrote X, but cargo check failed: …".
- **Analyzer gate**: end-of-turn (when the model stops calling tools), as today.

### 4.4 Gate integration (the moat, preserved)
- Injection + Identity: pre-flight on the model's prose/decision (as today).
- Permission(tier) + Slop&Safety: per `write_file` call, *before* execute — unchanged logic,
  new call-site (interceptor instead of post-parse).
- Compiler + Regression: after execute, folded into tool_result.
- All still emit `gate_verdict` events with `{level, reasons[]}`.

### 4.5 HITL staging inside the loop
- In `hitl`, `write_file` does **not** write: emit the `proposal` event and return a
  **synthetic `tool_result`** ("staged for review — will be applied on Accept") so the
  model can conclude the turn. The extension materializes on Accept (+ Post-Approval
  Compiler Gate + conflict-detection) exactly as today.

### 4.6 Streaming / protocol
- No `.proto` change: `IntentResponse{status,message,session_id}` is generic. Add new
  `status` values: `tool_call` (name+args requested) and `tool_result` (outcome). Prose
  tokens still stream via `token`.

### 4.7 Fallback & selection
- Config `tool_calling` (default **true** — shipped). When on: send `tools`.
- Auto-fallback to the tag path if: the endpoint errors on `tools`, OR the model returns
  no tool_calls but its prose contains a `<WRITE_FILE>` tag (belt-and-suspenders).
- The tag-path code stays intact and is used unchanged when fallback fires.
- System prompt: rely on the API `tools` field for tool guidance; keep the WRITE_FILE
  instructions only in the fallback branch.

## 5. Risks & mitigations
- **Runaway loop** → hard `max_tool_iters` cap + `tx.is_closed()` abort (as today).
- **Malformed tool args** → schema validation → error tool_result → model self-corrects.
- **Streaming tool_calls assembly** → assemble per `index`; unit-tested against captured
  LM Studio deltas.
- **Multi-write per turn + gates** → gates run per call; a blocked write returns an error
  result without aborting the batch.
- **HITL in a loop** → synthetic staged result (4.5); never writes pre-Accept.
- **Behaviour drift** → quick-bench must stay green through the tool path before flipping
  the default (4.7 / open Q1).

## 6. Implementation plan (verifiable slices)
- **Slice 0 — types** (`llm.rs`): extend ChatMessage/Request + delta tool_calls assembly.
  No loop change. Verify: builds, existing 26 tests pass, + unit test round-tripping a
  tool message and assembling tool_calls from delta fragments.
- **Slice 1 — loop** (`core.rs`): bounded agent loop + `write_file` tool + gates-as-
  interceptor + errors-as-tool-result + cap, behind `tool_calling` flag; tag fallback
  intact. Verify: `quick-bench` green via the tool path (`tool_calling=true`); A/B vs tag.
- **Slice 2 — streaming/UI**: `tool_call`/`tool_result` events + webview rendering.
- **Slice 3 — HITL staging** inside the loop (synthetic staged result + Accept materialize).
- **Later**: `read_file`, `edit` (oldText/newText + fuzzy + unified patch), `run` (with
  tier/permission + container thinking).

## 7. Resolved decisions (Fab, 2026-06-20)
1. `tool_calling` default = **ON (true)** — flipped 2026-06-20 after Slice 1+2 bench-green (commit 3561bf0).
2. **One `max_tool_iters` (default 6)** governs the tool path; `R` (=3) remains only for the tag fallback.
3. Tool guidance lives **only in the API `tools` field**; the WRITE_FILE prose stays in the fallback branch only.

See [[freecode-pi-reference]] for the source patterns and [[checkpoint-20260619]] for the
current build state.
