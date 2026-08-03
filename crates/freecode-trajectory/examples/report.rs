//! `cargo run -p freecode-trajectory --example report -- <claude-project-dir>...`
//!
//! READ-ONLY: extract trajectories from the Claude Code project dir(s) you pass and print an
//! aggregate report (outcomes, tool distribution, per-action success, RFC-005 learn candidates).
//! You choose the dirs — pass only projects you are free to mine.

use freecode_trajectory::{parse_claude_session, Trajectory};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn jsonl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                jsonl_files(&p, out);
            } else if p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                out.push(p);
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: report <claude-project-dir>...  (read-only; you pick the dirs)");
        return;
    }
    let mut trajectories: Vec<Trajectory> = Vec::new();
    for a in &args {
        let dir = Path::new(a);
        let project = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .replace("-Users-fab-Documents-git-", "")
            .replace("-Users-fab-", "~/");
        let mut files = Vec::new();
        jsonl_files(dir, &mut files);
        for f in files {
            if let Ok(s) = std::fs::read_to_string(&f) {
                trajectories.push(parse_claude_session(&s, &project));
            }
        }
    }
    report(&trajectories);
}

fn report(tr: &[Trajectory]) {
    let n = tr.len();
    let (mut ok, mut had_err, mut empty) = (0, 0, 0);
    let (mut tool_calls, mut errors) = (0usize, 0usize);
    let mut tools: BTreeMap<String, usize> = BTreeMap::new();
    for t in tr {
        match t.outcome {
            "ok" => ok += 1,
            "had-errors" => had_err += 1,
            _ => empty += 1,
        }
        tool_calls += t.tool_calls;
        errors += t.errors;
        for a in &t.actions {
            *tools.entry(a.tool.clone()).or_default() += 1;
        }
    }
    println!("trajectories: {n}  ({ok} ok · {had_err} had-errors · {empty} empty)");
    let acted = tool_calls.max(1);
    println!(
        "tool calls: {tool_calls}  ·  per-action success: {:.1}%",
        100.0 * (tool_calls - errors) as f64 / acted as f64
    );
    let mut by: Vec<_> = tools.into_iter().collect();
    by.sort_by_key(|t| std::cmp::Reverse(t.1));
    println!("top tools:");
    for (t, c) in by.into_iter().take(12) {
        println!("  {c:6}  {t}");
    }
    // RFC-005 learn candidates: did real work and finished clean.
    let mineable = tr.iter().filter(|t| t.outcome == "ok" && t.tool_calls >= 3).count();
    println!("learn candidates (ok + >=3 tool calls): {mineable}");
}
