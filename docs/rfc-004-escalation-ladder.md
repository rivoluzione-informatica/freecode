# RFC-004 — Gate-driven model escalation ladder

Status: **Slice 0 SHIPPED · deepened by [RFC-006](rfc-006-tiered-generate-and-validate.md)** · Date: 2026-06-20 (rev 2026-06-22) · Scope: `freecode-daemon` (routing)

> **Read [RFC-006](rfc-006-tiered-generate-and-validate.md) for the resolved architecture** — the
> data-grounded tier split (codegen 55.5% / audit 30.1% / distill 14%), the *generate-and-validate*
> decision, the validator stack, the typed verdict firewall, and the AIMP-as-glue decision (C/A/B).
> Shipped since this draft: the deterministic classifier with an **Audit** class (extracted to the
> shared `freecode-classify` crate), the `freecode-verdict` firewall, and **3-way router telemetry**
> (Slice 0 — drives nothing). RFC-006 §10 records which open questions below are now resolved.

## 1. Goal & tension

freecode's UX thesis (see the `freecode-ux-philosophy` memory): the user should NOT tune effort /
thinking / model / mode by hand. Every such knob exists because the system is uncertain and pushes
the decision onto the user. freecode's moat — it **verifies** with gates (compile / safety /
regression) — is exactly what lets it *remove* those knobs: pick a sensible default and let the gates
catch failures.

This RFC names the mechanism that replaces the knobs: a **tiered escalation ladder**. Start at the
cheapest capability that can plausibly do the work; **escalate only when a gate fails or confidence
is low**. Effort/model stop being sliders and become an automatic, outcome-driven ladder — and a
ladder grounded in verified results beats a slider grounded in the user's guess.

External proof points (grounded restudy, see `headroom-distill-restudy-20260620`):
- **headroom** = the deterministic half: line-importance + log/JSON transforms, free and instant.
  Already ported into `freecode-compress` (Tier 0 below) — RFC-003.
- **distill** (`samuelfaj/distill`) = the small-specialist half: a 1.7B MLX "Expert Language Model"
  that distills command output, 1:1 harness-driven, with a remote fallback. Proves a small local
  model can own a high-frequency job. (No confidence→escalate logic of its own — that is this RFC.)

## 2. Non-goals / ethos constraints

- **No manual model/effort knob in product UI.** The only user-facing axis stays autonomy
  (Suggest / Auto) per `freecode-ux-philosophy`. Tier selection is automatic.
- **Local-first.** Every tier below the top must run locally. The optional SLM tier is a *local*
  endpoint; no cloud in the default path.
- **Deterministic Tier 0 stays primary.** The ladder never replaces deterministic compression /
  transforms with a model — it adds model tiers *above* them.
- **Bounded escalation.** A hard cap on escalations per task; every escalation is logged with its
  trigger so the ladder is auditable (and tunable from data, not vibes).
- **Not** `freecode learn` (trajectory-mined memory proposals). RFC-003 §6 tentatively called that
  "RFC-004"; it is a distinct feature and is reassigned to a future **RFC-005**. This RFC is only
  the escalation ladder.

## 3. The ladder

| Tier | Capability | Cost | Owns |
|------|-----------|------|------|
| **T0** deterministic | `freecode-compress` (fit / log / json / memory hygiene) | ~0, instant | context shaping; never a knob |
| **T1** local SLM (optional) | a small local model (distill-style) | cheap, local | output distillation, trivial edits, classification/routing, "is this done?" checks |
| **T2** main model | the configured local/served LLM | the real cost | reasoning, codegen, anything T1 fails or is unsure about |

**Escalation triggers (T1→T2, and within T2 "try harder"):**
- a **gate fails** (compile error, safety flag, regression) → escalate and retry with more capability;
- **low confidence** (see §5) on the lower tier's output;
- task class is known-hard (codegen on a large diff) → start at T2 directly.

De-escalation: high-frequency, low-stakes work (summarize this log, classify this output, "did the
build pass?") starts at T0/T1 and never touches T2 unless a gate says so.

## 4. Design — where it plugs in

freecode already talks to a local LLM endpoint, and `core.rs` already runs gates and retries on
failure. The ladder is a **router** in front of the existing dispatch:

1. **Task classifier** (deterministic, cheap): map the turn to a class (chat-Q&A, output-distill,
   trivial-edit, codegen, …) → a starting tier. No model needed; a typed match like the tool
   dispatch.
2. **Tier dispatch**: T0 transforms always run (already wired). T1, if enabled, is just another
   local endpoint selected by the router. T2 is today's path.
3. **Gate-driven escalation**: reuse the existing gate verdicts. On failure, the router escalates the
   tier (or raises the main model's effort) and retries — the retry loop already exists; this RFC
   makes *what it escalates to* a function of the gate, not a constant.
4. **Telemetry**: log (task class, starting tier, escalations, final tier, gate outcome) so the
   default tier-per-class is learned from data.

The SLM tier mirrors distill's shape: a local OpenAI-compatible endpoint (distill spins up an MLX
server at `localhost:port`, `/v1/chat/completions`, with a concurrency gate). freecode would treat
it as one more endpoint in the router — not a new transport.

## 5. Open questions (for Fab)

1. **The SLM**: reuse `distill-1.7B-MLX` as-is, distill our own freecode-specialist, or skip T1
   initially and ship the ladder as T0→T2 only (gate-driven effort escalation) first?
2. **Confidence signal**: what feeds "low confidence"? Options — gate verdict only (simplest, and
   already trustworthy), a self-rated score from the lower tier, or output-shape heuristics. Lean
   gate-only to start (it is the moat).
3. **Task classifier**: hand-written typed rules first, or learned? Start typed (deterministic).
4. **Distill-language**: adopt distill's deterministic output DSL (`S/C/D/R/O/N/P` prefixes, macros,
   thread-local vars — zero model needed) as a freecode output convention now, independent of T1?
   It both saves tokens and is how a future freecode SLM would talk compactly.

## 6. Slices

- **Slice 0** — telemetry only: classify each turn + log where a gate-driven escalation *would*
  trigger. No behavior change. **DONE 2026-06-22**: `classify_task` (with the Audit class) +
  `log_turn_class`/`log_escalation_signal`, and the 3-way `log_route` over the `freecode-verdict`
  firewall (ship / retry-same-tier / escalate-to-T2). Drives nothing; measures the escalate band.
- **Slice 1** — router skeleton: task classifier + explicit tier dispatch (T0→T2), gates drive the
  existing retry's escalation. No SLM yet.
- **Slice 2** — confidence signal (start: gate-only) feeding escalation.
- **Slice 3 (optional)** — wire a local SLM tier (T1) for output-distill / trivial tasks behind a
  flag, measured against T2-only on latency and gate pass-rate.
- **Independent** — adopt the distill-language output convention (a system-prompt mode), gated like
  any other capability.

## 7. Relationship to other RFCs

- Builds on **RFC-003** (deterministic compression = Tier 0, shipped) and **RFC-001** (structured
  tool-calling, the dispatch the router sits in front of).
- `freecode learn` (trajectory mining → gated memory proposals) → future **RFC-005** (was loosely
  tagged RFC-004 in RFC-003 §6; reassigned here to avoid the collision).
- The whole RFC is the engineering form of `freecode-ux-philosophy`: gates let us delete the knobs.
