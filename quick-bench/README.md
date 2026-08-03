# picocoder quick-bench

A small, fast **"indicators every iteration"** harness — not a full eval. Run it
after a breaking change to see whether the agent still produces working,
slop-free edits, and to measure each deterministic gate's contribution.

It drives the daemon through the `freecode` CLI over fixture tasks and reuses the
daemon's own streamed `metrics` + `gate_verdict` events, so it needs almost no
new instrumentation.

## Indicators

| Indicator | Source |
|---|---|
| `assertion_pass_rate` | per-task file/content/response/gate asserts |
| `compile_pass_rate` | each run's `success` metric (Compiler Gate green) |
| `avg_attempt_count` | self-correction retries (`attempt_count`) |
| `avg_model_latency` / `avg_total_latency` | trajectory metrics |
| `tokens_per_success` | `completion_tokens` over successful runs |
| gate pass rates | `Identity Gate`, `Slop & Safety Gate`, `Compiler Gate` verdicts |

## Prerequisites

1. Daemon running: `cargo run --bin freecode-daemon`
2. A reachable OpenAI-compatible LLM endpoint.
3. Build the CLI once (faster than `cargo run` per task): `cargo build -p freecode-cli`

## Run

```bash
cd freecode/quick-bench

# validate the harness itself, offline (no daemon/LLM needed)
python3 run_bench.py --self-test

# run the suite (all gates on)
python3 run_bench.py --endpoint http://127.0.0.1:1234/v1/chat/completions --model gemma-4-e2b-it-mlx

# ablation: baseline + one-gate-off runs; prints Δ per gate
python3 run_bench.py --ablate --endpoint ... --model ...

# median-of-N to tame small-model noise
python3 run_bench.py --repeat 5 --endpoint ... --model ...

# protocol ablation: force the legacy <WRITE_FILE> tag path
# (the daemon default is now structured tool-calling; --tool-calling forces it on, now redundant)
python3 run_bench.py --no-tool-calling --endpoint ... --model ...
```

Results are written to `results/quick_bench_<timestamp>.json`; a summary table is
printed to stdout. Commit a results file to track regressions across iterations.

## Ablation (per-gate contribution)

`--ablate` re-runs the suite with one gate disabled at a time (via a
`.freecode/config.json` written into each task workspace: `safety_gate`,
`identity_gate`, `auto_verify`). A **positive Δ vs baseline** means that gate was
catching real problems — turning "we have gates" into "the Slop & Safety Gate is
worth +N% assertion pass-rate." (Mechanism from `docs-harness/2606.02373`.)

## Fixture schema (`tasks.jsonl`, one JSON object per line)

```jsonc
{
  "id": "rust_add_fn",
  "mode": "auto",                 // chat | hitl | auto
  "prompt": "…explicit instruction…",
  "seed": "mini-crate",           // optional dir under seeds/ copied into the workspace
  "assert": {
    "files_exist":   ["src/x.rs"],
    "contains":      {"src/x.rs": ["fn add"]},
    "not_contains":  {"src/x.rs": ["todo!"]},
    "response_contains":     ["FreeCode"],     // checks streamed model text
    "response_not_contains": ["gemma"],
    "gate_passed":   {"Compiler Gate": true},  // last verdict for that gate
    "success": true                            // run's success metric
  }
}
```

Each task runs in an isolated temp workspace with a fresh session id. Add fixtures
freely; keep prompts explicit to minimize model-to-model variance.

## Caveats

- **Small-model nondeterminism.** Local models vary run-to-run (latency and output
  length can swing 2–5×). For stable ablation deltas, prefer a larger model and/or
  use `--repeat N` (e.g. `--repeat 5`) to run each config N times and report medians —
  a single run per config is noisy.
- **Ablation asserts.** When a gate is disabled, the runner automatically drops any
  `gate_passed` assert that references *that* gate (it would fail tautologically),
  so only outcome-based asserts (file/content/response/success) measure the gate's
  real contribution. Give ablation-sensitive tasks an outcome assert that fails when
  the gate is off (e.g. the identity probe checks `response_not_contains`).
