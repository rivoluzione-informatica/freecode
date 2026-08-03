# RFC-003 — Deterministic context compression (in-process)

Status: **DRAFT (design only — no code)** · Date: 2026-06-20 · Scope: `freecode-daemon` (+ quick-bench)

## 1. Goal & tension

freecode drives **small local models** (gemma-4-e2b) with a **tight context window**. Today the
context budget is handled crudely:
- `read_file` output is HEAD-truncated to 20000 chars (`core.rs` ~570).
- failed-compile output is HEAD-truncated to 8000 chars (`core.rs` ~703).
- `trim_history` budgets by `content.len()` **bytes** against a magic **48000** (`core.rs` ~1955).
- the workspace overview (`scanner.rs`) is rebuilt and **resent every turn** — measured at
  **~2400 tokens/turn** on the freecode repo (prompt_tokens ≈2700 here vs ≈295 on an empty
  workspace), uncompressed, never cached cross-turn.

HEAD-truncation is the worst offender: **rustc/tsc dump warnings first and the real
`error[E0599]` line often sits past the cut → the model never sees the error it must fix.**

Goal: replace blind truncation with **deterministic, error-aware selection** that keeps the
lines the model needs, and budget by real tokens — **without** importing a heavy ML/cloud stack.

This RFC is the design; implementation waits for sign-off (per RFC-001/002 rigor).

## 2. Non-goals / ethos constraints

- **Vendor individual files, NOT the `headroom-core` crate.** The crate drags tiktoken-rs, HF
  tokenizers, hf-hub, fastembed/ONNX-Runtime (~30 MB BAAI/bge auto-download), magika(ONNX),
  rusqlite, redis, rayon — every one violates local-first / no-cloud / small-footprint /
  determinism. The useful files were verified to depend ONLY on **aho-corasick, regex, flate2,
  md-5, serde_json** (all already in freecode's `Cargo.lock`). headroom is Apache-2.0 → vendor
  with attribution.
- **No retrieval handshake (CCR).** Small models won't reliably issue a `retrieve(hash)` call,
  turning "lossless" into silently-lost context. Skip.
- **No proxy / MCP integration.** The right seam for an in-process Rust daemon talking to a local
  endpoint is a direct function call, not a process boundary.
- **Deterministic + testable.** Same input → same output. No model, no network.

## 3. Design

### 3.1 Seam — `daemon/src/compress.rs`
One module, one entry point. In `core.rs`, every tool result is built into `result_text` and
pushed as a `role:"tool"` message. Route that text through `compress::fit(text, kind, budget)`
**before** the push, where `kind` is known at the call site (read_file → Source, compile →
BuildLog, edit → n/a). No content sniffing needed (freecode's tools are typed — a 3-line match,
not headroom's regex/magika detector).

### 3.2 What we vendor (the two core wins)
- **W1 line-importance selection** — port `signals/{line_importance,keyword_detector}.rs`
  (aho-corasick only; drop the `Tiered` trait). `score(line, ctx)` → priority. When over budget,
  keep highest-priority lines (Error≈0.95, Security≈0.85, Warning≈0.75) **plus a ±N context
  window**, instead of the first N chars. Adopt headroom's documented keyword fixes verbatim:
  drop `token` from the security set (false-positives on "LLM token"); add
  `abort/timeout/denied/rejected` to the error set.
- **W2 token estimator** — port `tokenizer/estimator.rs` (~30 LOC, zero deps): counts Unicode
  **scalars** not bytes, `tokens ≈ max(1, chars/cpt + 0.5)`, cpt≈3.5–4.0. Add
  `compress::estimate_tokens(&str)`; `trim_history` and the truncation thresholds budget by
  estimated **tokens** vs a configurable window, not 48000 bytes.

### 3.3 ctx-bench (measurement — the "where do we stand" harness)
A small Rust test/bin (`quick-bench` companion) modeled on headroom's benchmark, scoped to what
freecode ingests **today**:
- **Fixtures**: (a) a long cargo error log with the real `error[E…]` deliberately past 8000
  chars; (b) a large source file read; (c) a synthetic history for `trim_history`. (Shell/grep
  fixtures deferred until `run`/MCP land — RFC-002.)
- **Metrics per fixture**: raw tokens → tokens actually emitted to the model → **critical-info
  retained? (boolean: does the error line / target symbol survive?)** → latency.
- **Output**: a headroom-style table. Baseline run proves the failure (error line LOST under
  head-truncation); the same harness then measures the W1/W2 gain (error RETAINED, tokens ≤).
- Also instrument prompt-token **composition** (system / overview / memories / history) so the
  ~2400-tok/turn overview cost is visible and trackable.

### 3.4 Memory budget hygiene (W3)
Today `core.rs` injects top-5 project + top-5 global BM25 memories concatenated with **no cap,
no dedup, no staleness check**. Add three deterministic passes (all stdlib/regex, no embeddings):
hard token cap on the combined block; Jaccard word-set dedup (~0.85) to drop near-duplicates;
git-staleness prune (skip a memory referencing a `path/that.rs` that no longer exists — reuse
`git.rs`). Keep freecode's BM25 IDF (it is better than headroom's degenerate `ln(2)`); optionally
borrow only headroom's UUID/long-token tokenization so identifiers/error-codes survive.

### 3.5 Config & default
New `GateConfig.compression` flag. Shipped **Default OFF**, then **flipped to Default ON
(2026-06-20)** once ctx-bench was green on synthetic AND real data (no panics; safe degradation) —
same evidence-gated discipline as RFC-001. Set `false` in `.freecode/config.json` for ablation.

## 4. Slices
- **Slice 0** — `ctx-bench` harness + fixtures + **baseline numbers** (proves the head-truncation
  loss). No behavior change.
- **Slice 1** — W2 token estimator; `trim_history` + thresholds budget by tokens. Measure.
- **Slice 2** — W1 `compress.rs` line-importance; route read_file + compile-error through it
  behind `compression` flag. Re-run ctx-bench → before/after.
- **Slice 3** — W3 memory budget hygiene.
- **Slice 4 (later)** — adopt `adaptive_sizer::compute_optimal_k` (knee-point sizing, flate2+md5)
  to replace fixed caps; `diff_compressor` core (~200 LOC) for the edit-retry diff flow.

## 5. Correctness & determinism
- **Invariant**: compression must NEVER drop a line the importance scorer marks Error/Security.
  Unit-tested with the cargo-error-past-8k fixture (the exact failure this RFC targets).
- Pure functions, snapshot tests; the esbuild-style "passes locally == passes" guarantee.
- Truncation stays a safe fallback: if scoring yields nothing, head-truncate as today (no regression).

## 6. Out of scope (tracked elsewhere)
- **W4 `freecode learn`** (mine trajectories → gated memory proposals) — high value but a distinct
  feature; deserves its own **RFC-005**. See [[freecode-headroom-integration]]. (RFC-004 is the
  gate-driven escalation ladder; `freecode learn` was reassigned to RFC-005 to avoid the collision.)
- **W6 Rust hygiene** (rust-toolchain pin, deny.toml, pre-commit, CI) — pure chores, no RFC needed.
- CCR retrieve, proxy, MCP, embedding/hybrid relevance, smart_crusher, providers/telemetry — **skip**
  (§2 + [[freecode-headroom-integration]] for the why).

## 7. Open questions (for Fab)
1. **ctx window source** — hardcode a conservative default (e.g. 8k/32k) or read the model's
   `num_ctx` from config? (estimator needs a target to budget against.)
2. **`compression` default** — opt-in→flip-after-bench (RFC-001 style), or straight ON since it
   degrades to head-truncation safely?
3. **Scope of Slice 0** — ship ctx-bench standalone first (just to see the baseline), or bundle
   with Slice 1 so the first PR already shows a delta?
4. **W4 learn** — spin up RFC-005 now, or finish compression (this RFC) first?

See [[freecode-headroom-integration]] (the full ranked scan + skips), [[freecode-roadmap]],
and `docs/rfc-002-run-tool.md` (shell/grep outputs that will extend §3.3's fixtures once `run` lands).
