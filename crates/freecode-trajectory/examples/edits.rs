//! `cargo run -p freecode-trajectory --example edits -- <out.jsonl> <claude-project-dir>...`
//!
//! READ-ONLY: mine RFC-006 Battle-1 AST edit pairs (`before → after` + the local intent) from the
//! Edit/MultiEdit tool calls in the dirs you pass. One JSON object per line — the narrow training set
//! for freelm ("given AST context + intent, produce the tree edit"). You choose the dirs; the corpus
//! may include work you are not free to mine, so build the allowlist deliberately.

use freecode_trajectory::parse_claude_edit_pairs;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_CHARS: usize = 2000; // each side of an edit must be <= this (local edit, not a rewrite)

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
        eprintln!("usage: edits <out.jsonl> <claude-project-dir>...  (read-only; you pick the dirs)");
        std::process::exit(2);
    }
    let out_path = &args[0];
    let mut f = std::fs::File::create(out_path).expect("create output file");
    let (mut written, mut bytes, mut before_chars, mut after_chars) = (0usize, 0usize, 0usize, 0usize);
    let mut by_lang: BTreeMap<&'static str, usize> = BTreeMap::new();

    for d in &args[1..] {
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
            for p in parse_claude_edit_pairs(&s, &project, MAX_CHARS) {
                let line = serde_json::to_string(&p).expect("serialize");
                bytes += line.len() + 1;
                before_chars += p.before.chars().count();
                after_chars += p.after.chars().count();
                *by_lang.entry(p.lang).or_insert(0) += 1;
                writeln!(f, "{line}").expect("write");
                written += 1;
            }
        }
    }

    let avg = |t: usize, n: usize| t.checked_div(n).unwrap_or(0);
    println!(
        "wrote {written} Battle-1 edit pairs → {out_path}  ({:.1} MB)   avg before/after = {}/{} chars",
        bytes as f64 / 1e6,
        avg(before_chars, written),
        avg(after_chars, written)
    );
    println!("  by language:");
    let mut langs: Vec<_> = by_lang.into_iter().collect();
    langs.sort_by_key(|l| std::cmp::Reverse(l.1));
    for (lang, n) in langs {
        println!("    {lang:<12}: {n:>6}  ({:.1}%)", 100.0 * n as f64 / written.max(1) as f64);
    }
}
