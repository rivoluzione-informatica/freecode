#!/usr/bin/env python3
"""
picocoder quick-bench — fast "indicators every iteration" harness.

It drives the FreeCode daemon through the `freecode` CLI over a set of small
fixture tasks and reports deterministic indicators by reusing the daemon's own
streamed metrics + gate verdicts:

  * compile_pass_rate   (from each run's `success` metric)
  * gate pass rates     (Identity / Slop & Safety / Compiler verdicts)
  * assertion pass rate (file/content/response asserts per task)
  * avg self-correction attempts, latency, tokens-per-success

Prereqs to run for real: the daemon must be up (`cargo run --bin freecode-daemon`)
and an OpenAI-compatible LLM endpoint reachable. Build the CLI once for speed:
`cargo build -p freecode-cli`.

Usage:
  python3 run_bench.py                      # run the suite once (all gates on)
  python3 run_bench.py --ablate             # baseline + one-gate-off runs, report deltas
  python3 run_bench.py --self-test          # validate the harness logic offline (no daemon)
  python3 run_bench.py --endpoint http://127.0.0.1:1234/v1/chat/completions --model gemma-4-e2b-it-mlx
"""
import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(HERE)  # freecode/  (cargo workspace root)


# ----------------------------------------------------------------------------
# Transcript parsing — the CLI prints `[metrics] {json}`, `[gate_verdict] {json}`,
# `[status] ...`, `[Step] ...`, and raw model tokens inline.
# ----------------------------------------------------------------------------
def parse_transcript(text):
    metrics = None
    gates = []
    response_lines = []
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("[metrics] "):
            try:
                metrics = json.loads(stripped[len("[metrics] "):])
            except json.JSONDecodeError:
                pass
        elif stripped.startswith("[gate_verdict] "):
            try:
                gates.append(json.loads(stripped[len("[gate_verdict] "):]))
            except json.JSONDecodeError:
                pass
        elif stripped.startswith("[status] ") or stripped.startswith("[Step] ") \
                or stripped.startswith("[files_read] ") or stripped.startswith("[proposal] ") \
                or stripped.startswith("[metrics]") or stripped.startswith("[gate_verdict]"):
            # control/event line — not response text
            continue
        else:
            # streamed model tokens / final-message continuation lines
            response_lines.append(line)
    return {"metrics": metrics, "gates": gates, "response": "\n".join(response_lines)}


def last_gate(gates, name):
    """Most recent verdict for a given gateName, or None."""
    found = None
    for g in gates:
        if g.get("gateName") == name:
            found = g
    return found


def evaluate(workspace, asserts, parsed):
    """Return (passed: bool, reasons: [str])."""
    reasons = []

    for rel in asserts.get("files_exist", []):
        if not os.path.isfile(os.path.join(workspace, rel)):
            reasons.append(f"missing file: {rel}")

    for rel, needles in asserts.get("contains", {}).items():
        path = os.path.join(workspace, rel)
        body = _read(path)
        if body is None:
            reasons.append(f"contains: missing file {rel}")
            continue
        for n in needles:
            if n not in body:
                reasons.append(f"{rel} missing substring: {n!r}")

    for rel, needles in asserts.get("not_contains", {}).items():
        body = _read(os.path.join(workspace, rel))
        if body is None:
            continue
        for n in needles:
            if n in body:
                reasons.append(f"{rel} contains forbidden substring: {n!r}")

    resp = parsed.get("response", "") or ""
    low = resp.lower()
    for n in asserts.get("response_contains", []):
        if n.lower() not in low:
            reasons.append(f"response missing: {n!r}")
    for n in asserts.get("response_not_contains", []):
        if n.lower() in low:
            reasons.append(f"response contains forbidden: {n!r}")

    for name, want in asserts.get("gate_passed", {}).items():
        g = last_gate(parsed.get("gates", []), name)
        if g is None:
            reasons.append(f"gate {name!r} never reported")
        elif bool(g.get("passed")) != bool(want):
            reasons.append(f"gate {name!r} passed={g.get('passed')} (wanted {want})")

    if "success" in asserts:
        m = parsed.get("metrics") or {}
        if bool(m.get("success")) != bool(asserts["success"]):
            reasons.append(f"success={m.get('success')} (wanted {asserts['success']})")

    return (len(reasons) == 0, reasons)


def _read(path):
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return None


# ----------------------------------------------------------------------------
# Running tasks
# ----------------------------------------------------------------------------
def resolve_cli(cli_arg):
    if cli_arg:
        return cli_arg.split()
    built = os.path.join(REPO_ROOT, "target", "debug", "freecode-cli")
    if os.path.isfile(built):
        return [built]
    return ["cargo", "run", "-q", "-p", "freecode-cli", "--"]


# Disabling a gate makes its own `gate_passed` assert tautologically fail, which
# would fake a positive ablation delta. Drop those asserts when the gate is off so
# only outcome-based assertions (file/content/response/success) reveal the gate's
# true contribution.
GATE_BY_FLAG = {
    "safety_gate": "Slop & Safety Gate",
    "identity_gate": "Identity Gate",
    "auto_verify": "Compiler Gate",
}


def asserts_for(task, config_overrides):
    asserts = task.get("assert", {})
    disabled = [GATE_BY_FLAG[k] for k, v in (config_overrides or {}).items()
                if v is False and k in GATE_BY_FLAG]
    if disabled and "gate_passed" in asserts:
        gp = {k: v for k, v in asserts["gate_passed"].items() if k not in disabled}
        asserts = {**asserts, "gate_passed": gp}
    return asserts


def run_task(task, opts, config_overrides):
    workspace = tempfile.mkdtemp(prefix=f"qb_{task['id']}_")
    try:
        seed = task.get("seed")
        if seed:
            seed_dir = os.path.join(HERE, "seeds", seed)
            if os.path.isdir(seed_dir):
                shutil.copytree(seed_dir, workspace, dirs_exist_ok=True)
        if getattr(opts, "tool_calling", False):
            config_overrides = {**(config_overrides or {}), "tool_calling": True}
        elif getattr(opts, "no_tool_calling", False):
            config_overrides = {**(config_overrides or {}), "tool_calling": False}
        if config_overrides:
            cfg_dir = os.path.join(workspace, ".freecode")
            os.makedirs(cfg_dir, exist_ok=True)
            with open(os.path.join(cfg_dir, "config.json"), "w") as f:
                json.dump(config_overrides, f)

        mode = task.get("mode", opts.mode_default)
        cmd = resolve_cli(opts.cli) + [
            "ask", task["prompt"],
            "--mode", mode,
            "--workspace", workspace,
            "--session", f"bench_{task['id']}_{int(time.time()*1000)}",
        ]
        if opts.endpoint:
            cmd += ["--endpoint", opts.endpoint]
        if opts.model:
            cmd += ["--model", opts.model]

        try:
            proc = subprocess.run(
                cmd, capture_output=True, text=True, timeout=opts.timeout,
                cwd=REPO_ROOT,
            )
            transcript = (proc.stdout or "") + "\n" + (proc.stderr or "")
        except subprocess.TimeoutExpired as e:
            transcript = (e.stdout or "") if isinstance(e.stdout, str) else ""
            parsed = parse_transcript(transcript)
            return {"id": task["id"], "passed": False, "reasons": ["timeout"],
                    "metrics": parsed["metrics"], "gates": parsed["gates"]}

        parsed = parse_transcript(transcript)
        passed, reasons = evaluate(workspace, asserts_for(task, config_overrides), parsed)
        return {"id": task["id"], "passed": passed, "reasons": reasons,
                "metrics": parsed["metrics"], "gates": parsed["gates"]}
    finally:
        if not opts.keep:
            shutil.rmtree(workspace, ignore_errors=True)


def aggregate(results):
    n = len(results) or 1
    passed = sum(1 for r in results if r["passed"])
    successes = [r for r in results if (r.get("metrics") or {}).get("success")]
    def avg(key):
        vals = [(r.get("metrics") or {}).get(key) for r in results]
        vals = [v for v in vals if isinstance(v, (int, float))]
        return round(sum(vals) / len(vals), 3) if vals else None
    tokens = [(r.get("metrics") or {}).get("completion_tokens") for r in successes]
    tokens = [t for t in tokens if isinstance(t, (int, float))]
    return {
        "tasks": len(results),
        "assertion_pass_rate": round(passed / n, 3),
        "compile_pass_rate": round(len(successes) / n, 3),
        "avg_attempt_count": avg("attempt_count"),
        "avg_model_latency": avg("model_latency"),
        "avg_total_latency": avg("total_latency"),
        "tokens_per_success": round(sum(tokens) / len(successes), 1) if successes and tokens else None,
    }


def median_summary(run_summaries):
    """Median of each numeric indicator across repeated runs (None if all None)."""
    keys = ["tasks", "assertion_pass_rate", "compile_pass_rate", "avg_attempt_count",
            "avg_model_latency", "avg_total_latency", "tokens_per_success"]
    out = {}
    for k in keys:
        nums = [s.get(k) for s in run_summaries if isinstance(s.get(k), (int, float))]
        out[k] = round(statistics.median(nums), 3) if nums else None
    return out


def print_run(label, med, task_pass_counts, repeat):
    suffix = f"  (median of {repeat} runs)" if repeat > 1 else ""
    print(f"\n=== {label}{suffix} ===")
    for k, v in med.items():
        print(f"  {k:24} {v}")
    for tid, cnt in task_pass_counts.items():
        mark = "✓" if cnt == repeat else ("~" if cnt > 0 else "✗")
        print(f"  {mark} {tid}: {cnt}/{repeat}")


# ----------------------------------------------------------------------------
# Self-test (no daemon) — validates parsing + assertion logic deterministically.
# ----------------------------------------------------------------------------
def self_test():
    print("Running quick-bench self-test (offline)...")
    transcript = "\n".join([
        "thinking...",
        "Here is your file.",
        '[gate_verdict] {"gateName":"Identity Gate","passed":true,"level":"none","reasons":[]}',
        '[gate_verdict] {"gateName":"Slop & Safety Gate","passed":true,"level":"none","reasons":[]}',
        '[metrics] {"success":true,"attempt_count":1,"model_latency":0.5,"total_latency":0.9,"completion_tokens":120}',
        "[status] Done. Actions Executed:",
        "wrote file",
    ])
    parsed = parse_transcript(transcript)
    assert parsed["metrics"]["success"] is True, "metrics not parsed"
    assert len(parsed["gates"]) == 2, f"expected 2 gates, got {len(parsed['gates'])}"
    assert last_gate(parsed["gates"], "Identity Gate")["passed"] is True

    ws = tempfile.mkdtemp(prefix="qb_selftest_")
    try:
        os.makedirs(os.path.join(ws, "src"), exist_ok=True)
        with open(os.path.join(ws, "src", "mathx.rs"), "w") as f:
            f.write("pub fn add(a: i32, b: i32) -> i32 { a + b }\n")

        ok, reasons = evaluate(ws, {
            "files_exist": ["src/mathx.rs"],
            "contains": {"src/mathx.rs": ["fn add", "a + b"]},
            "not_contains": {"src/mathx.rs": ["todo!"]},
            "gate_passed": {"Identity Gate": True, "Slop & Safety Gate": True},
            "success": True,
        }, parsed)
        assert ok, f"positive case should pass, got: {reasons}"

        bad_ok, bad_reasons = evaluate(ws, {
            "files_exist": ["src/missing.rs"],
            "gate_passed": {"Compiler Gate": True},  # never reported
        }, parsed)
        assert not bad_ok, "negative case should fail"
        assert any("missing file" in r for r in bad_reasons)
        assert any("never reported" in r for r in bad_reasons)
    finally:
        shutil.rmtree(ws, ignore_errors=True)

    summary = aggregate([
        {"id": "a", "passed": True, "metrics": {"success": True, "attempt_count": 1, "completion_tokens": 100}},
        {"id": "b", "passed": False, "metrics": {"success": False, "attempt_count": 3}},
    ])
    assert summary["assertion_pass_rate"] == 0.5
    assert summary["compile_pass_rate"] == 0.5

    med = median_summary([
        {"assertion_pass_rate": 1.0, "avg_attempt_count": 1.0, "tokens_per_success": None},
        {"assertion_pass_rate": 0.0, "avg_attempt_count": 3.0, "tokens_per_success": None},
        {"assertion_pass_rate": 1.0, "avg_attempt_count": 2.0, "tokens_per_success": None},
    ])
    assert med["assertion_pass_rate"] == 1.0  # median of [1,0,1]
    assert med["avg_attempt_count"] == 2.0    # median of [1,3,2]
    assert med["tokens_per_success"] is None  # all None -> None

    print("OK — self-test passed (parsing, assertions, aggregation, medians).")
    return 0


# ----------------------------------------------------------------------------
def load_tasks(path):
    tasks = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#"):
                tasks.append(json.loads(line))
    return tasks


def save_results(out_dir, payload, stamp):
    os.makedirs(out_dir, exist_ok=True)
    path = os.path.join(out_dir, f"quick_bench_{stamp}.json")
    with open(path, "w") as f:
        json.dump(payload, f, indent=2)
    print(f"\nResults written to {path}")


def main():
    ap = argparse.ArgumentParser(description="picocoder quick-bench")
    ap.add_argument("--tasks", default=os.path.join(HERE, "tasks.jsonl"))
    ap.add_argument("--cli", default="", help="CLI invocation (default: built binary or cargo run)")
    ap.add_argument("--endpoint", default="")
    ap.add_argument("--model", default="")
    ap.add_argument("--mode-default", default="auto")
    ap.add_argument("--timeout", type=float, default=120.0)
    ap.add_argument("--out", default=os.path.join(HERE, "results"))
    ap.add_argument("--ablate", action="store_true", help="run baseline + one-gate-off, report deltas")
    ap.add_argument("--repeat", type=int, default=1, help="run the suite N times per config; report medians (tames small-model noise)")
    ap.add_argument("--tool-calling", action="store_true", help="force tool_calling=true in each workspace (RFC-001 tool path; now the daemon default)")
    ap.add_argument("--no-tool-calling", action="store_true", help="force tool_calling=false (tag-path ablation; the daemon default is now ON)")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--keep", action="store_true", help="keep temp workspaces")
    opts = ap.parse_args()

    if opts.self_test:
        return self_test()

    tasks = load_tasks(opts.tasks)
    stamp = time.strftime("%Y%m%dT%H%M%S")

    configs = [("baseline", {})]
    if opts.ablate:
        configs += [
            ("no_safety_gate", {"safety_gate": False}),
            ("no_identity_gate", {"identity_gate": False}),
            ("no_auto_verify", {"auto_verify": False}),
        ]

    repeat = max(1, opts.repeat)
    report = {"stamp": stamp, "endpoint": opts.endpoint, "model": opts.model, "repeat": repeat, "runs": {}}
    baseline_med = None
    for label, overrides in configs:
        run_summaries = []
        task_pass_counts = {t["id"]: 0 for t in tasks}
        for _ in range(repeat):
            results = [run_task(t, opts, overrides) for t in tasks]
            run_summaries.append(aggregate(results))
            for r in results:
                if r["passed"]:
                    task_pass_counts[r["id"]] += 1
        med = median_summary(run_summaries)
        report["runs"][label] = {
            "median": med,
            "run_summaries": run_summaries,
            "task_pass_counts": task_pass_counts,
            "repeat": repeat,
        }
        print_run(label, med, task_pass_counts, repeat)
        if label == "baseline":
            baseline_med = med
        elif baseline_med:
            d_assert = round((baseline_med.get("assertion_pass_rate") or 0) - (med.get("assertion_pass_rate") or 0), 3)
            d_compile = round((baseline_med.get("compile_pass_rate") or 0) - (med.get("compile_pass_rate") or 0), 3)
            print(f"  Δ vs baseline (median): assertion {d_assert:+}, compile {d_compile:+}  "
                  f"(positive = this gate was contributing)")

    save_results(opts.out, report, stamp)
    return 0


if __name__ == "__main__":
    sys.exit(main())
