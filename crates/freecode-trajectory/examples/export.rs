//! `cargo run -p freecode-trajectory --example export -- <out.jsonl> <claude-project-dir>...`
//!
//! READ-ONLY export of SFT-ready conversations (chat `messages[]`, truncated + secret-redacted) for
//! the learn-candidate trajectories (outcome ok + >= 3 tool calls) in the dirs you pass. One JSON
//! object per line — ready to feed an SLM fine-tune (freelm / distill-style). You choose the dirs:
//! the local corpus may include work you are not free to mine, so build the allowlist deliberately.

use freecode_trajectory::parse_claude_conversation;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_CHARS: usize = 2000; // per-message truncation (tool results / args)
const MIN_ACTIONS: usize = 3; // learn-candidate threshold

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
    if args.len() < 2 {
        eprintln!("usage: export <out.jsonl> <claude-project-dir>...  (read-only; you pick the dirs)");
        std::process::exit(2);
    }
    let out_path = &args[0];
    let dirs = &args[1..];

    let mut f = std::fs::File::create(out_path).expect("create output file");
    // Sidecar with ONLY the rare conversational (>=2 human turns) subset — the high-value
    // chat-style slice you'll want to upsample against the agentic bulk at train time.
    let conv_path = format!("{out_path}.conversational.jsonl");
    let mut cf = std::fs::File::create(&conv_path).expect("create conversational file");
    let (mut written, mut skipped, mut bytes) = (0usize, 0usize, 0usize);
    let (mut agentic, mut conversational, mut conv_bytes, mut agentic_actions, mut conv_actions) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    // RFC-004 task-class distribution (via the shared freecode-classify the daemon's router uses).
    let mut class_dist: BTreeMap<&'static str, usize> = BTreeMap::new();

    for d in dirs {
        let dir = Path::new(d);
        let project = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .replace("-Users-fab-Documents-git-", "")
            .replace("-Users-fab-", "~/");
        let mut files = Vec::new();
        jsonl_files(dir, &mut files);
        for file in files {
            let Ok(s) = std::fs::read_to_string(&file) else { continue };
            let conv = parse_claude_conversation(&s, &project, MAX_CHARS);
            if conv.outcome == "ok" && conv.action_count >= MIN_ACTIONS {
                let line = serde_json::to_string(&conv).expect("serialize");
                bytes += line.len() + 1;
                writeln!(f, "{line}").expect("write");
                written += 1;
                *class_dist.entry(conv.task_class).or_insert(0) += 1;
                if conv.mode == "conversational" {
                    conversational += 1;
                    conv_actions += conv.action_count;
                    conv_bytes += line.len() + 1;
                    writeln!(cf, "{line}").expect("write conversational");
                } else {
                    agentic += 1;
                    agentic_actions += conv.action_count;
                }
            } else {
                skipped += 1;
            }
        }
    }

    let avg = |tot: usize, n: usize| if n == 0 { 0.0 } else { tot as f64 / n as f64 };
    println!(
        "wrote {written} SFT conversations → {out_path}  ({:.1} MB)   [{skipped} skipped: not ok / < {MIN_ACTIONS} actions]",
        bytes as f64 / 1e6
    );
    println!("  mode split (tagged in each record as \"mode\"):");
    println!(
        "    agentic        (1 human turn) : {agentic:>5}  ·  {:.1} avg tool-calls",
        avg(agentic_actions, agentic)
    );
    println!(
        "    conversational (>=2 turns)    : {conversational:>5}  ·  {:.1} avg tool-calls  ({:.1} MB) → {conv_path}",
        avg(conv_actions, conversational),
        conv_bytes as f64 / 1e6
    );
    println!("  task-class split (tagged as \"task_class\", via freecode-classify = the daemon's router):");
    let mut classes: Vec<_> = class_dist.into_iter().collect();
    classes.sort_by_key(|c| std::cmp::Reverse(c.1));
    for (cls, n) in classes {
        println!("    {cls:<16}: {n:>5}  ({:.1}%)", 100.0 * n as f64 / written.max(1) as f64);
    }
}
