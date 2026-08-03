//! ctx-bench — RFC-003: measure freecode's context truncation, BASELINE vs line-importance
//! compression. Proves the problem and the fix in one table:
//!   - BASELINE (current head-truncation): keeps the first N chars → the critical line
//!     (rustc error / asked-about symbol) sits past the cut and is LOST.
//!   - COMPRESSED (freecode-compress::fit): keeps the highest-importance / query-matched
//!     lines within the same budget → critical line KEPT.
//!
//! Run: `cargo run -p freecode-daemon --example ctx_bench`

use freecode_compress::{compress, estimate_tokens, Kind};

/// freecode's CURRENT behavior at the read_file / compile-error seams in core.rs.
fn head_truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        s.chars().take(max_chars).collect::<String>() + "\n…[truncated]"
    } else {
        s.to_string()
    }
}

struct Fixture {
    name: &'static str,
    /// char cap for the baseline head-truncation (mirrors the core.rs fallback seam).
    budget_chars: usize,
    /// token budget for `fit` (mirrors the core.rs compression seam, RFC-003 W2).
    budget_tokens: usize,
    content: String,
    critical: &'static str,
    /// None → Log (compile); Some(q) → Source read_file with the turn's query.
    query: Option<&'static str>,
    /// true → JSON read → compress_json (smart-crusher sampling) instead of fit/compress_log.
    is_json: bool,
    /// true → diff read → compress_diff (hunk-trim) instead of the others.
    is_diff: bool,
}

fn make_big_json() -> String {
    let mut s = String::from("[");
    for i in 0..500 {
        if i > 0 {
            s.push(',');
        }
        let name = if i == 499 { "FINAL_MARKER" } else { "item" };
        s.push_str(&format!(r#"{{"id":{},"name":"{} {}","ok":true}}"#, i, name, i));
    }
    s.push(']');
    s
}

fn make_cargo_log() -> String {
    let mut s = String::new();
    for i in 0..320 {
        s.push_str(&format!(
            "warning: unused variable: `tmp{}`\n  --> src/lib.rs:{}:9\n",
            i,
            i + 10
        ));
    }
    s.push_str("error[E0599]: no method named `frobnicate` found for struct `Widget`\n  --> src/lib.rs:402:18\n");
    s
}

fn make_big_source(target: &str) -> String {
    let mut s = String::new();
    for i in 0..900 {
        s.push_str(&format!("// boilerplate line {}\nfn helper_{}() {{}}\n", i, i));
    }
    s.push_str(&format!(
        "pub fn {}() {{ /* the exact function the model was asked about */ }}\n",
        target
    ));
    s
}

fn make_big_diff(critical: &str) -> String {
    let mut s = String::new();
    for f in 0..30 {
        s.push_str(&format!("diff --git a/file{}.rs b/file{}.rs\n", f, f));
        s.push_str(&format!("--- a/file{}.rs\n+++ b/file{}.rs\n", f, f));
        s.push_str(&format!("@@ -1,9 +1,9 @@ fn f{}()\n", f));
        for c in 0..6 {
            s.push_str(&format!(" unchanged context line {} of file {}\n", c, f));
        }
        if f == 29 {
            // the file you care about = the heaviest hunk → survives the caps.
            for k in 0..5 {
                s.push_str(&format!("-old crit {}\n", k));
            }
            s.push_str(&format!("+{}\n", critical));
        } else {
            s.push_str(&format!("-old line {}\n+new line {}\n", f, f));
        }
    }
    s
}

/// `cargo run --example ctx_bench -- <file>...` → run the real compression path over real files
/// (the "real-data bench" gating the compression-default flip). Picks the compressor the way the
/// core.rs seams do: .json → compress_json, error/warning-heavy → compress_log, else → fit.
fn run_real_files(paths: &[String]) {
    println!("ctx-bench — REAL files (production compressors, compression seam budgets)\n");
    println!("{:<46} {:>8} {:>8} {:>7}  via", "file", "raw tok", "comp tok", "kept%");
    println!("{}", "-".repeat(82));
    for p in paths {
        let content = match std::fs::read_to_string(p) {
            Ok(c) => c,
            Err(e) => {
                println!("{p}: read error: {e}");
                continue;
            }
        };
        let raw = estimate_tokens(&content);
        // Route through the unified pipeline (same as the core.rs seams).
        let looks_log = content.lines().take(500).any(|l| l.contains("error[") || l.contains("warning:"));
        let (comp, via) = if looks_log {
            (compress(&content, Kind::BuildLog, 2200), "build-log")
        } else {
            (compress(&content, Kind::File { query: None, path: Some(p) }, 6000), "file")
        };
        let ct = estimate_tokens(&comp);
        let pct = if raw > 0 { ct as f64 / raw as f64 * 100.0 } else { 100.0 };
        let shown = if p.len() > 46 { &p[p.len() - 46..] } else { p };
        println!("{:<46} {:>8} {:>8} {:>6.0}%  {}", shown, raw, ct, pct, via);
    }
    println!("\n(no panics above = the real-data path is safe; kept% < 100 on large files = the win)");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        run_real_files(&args);
        return;
    }

    let fixtures = vec![
        Fixture {
            name: "cargo error log (error past 8k)",
            budget_chars: 8000,
            budget_tokens: 2200, // ≈ 8000 chars — the core.rs compile seam
            content: make_cargo_log(),
            critical: "error[E0599]",
            query: None,
            is_json: false,
            is_diff: false,
        },
        Fixture {
            name: "large source read (symbol near end)",
            budget_chars: 20000,
            budget_tokens: 6000, // ≈ 20000 chars — the core.rs read_file seam
            content: make_big_source("targetSymbol"),
            critical: "pub fn targetSymbol",
            query: Some("where is targetSymbol defined and what does it do"),
            is_json: false,
            is_diff: false,
        },
        Fixture {
            name: "large JSON array read (record near end)",
            budget_chars: 4000,
            budget_tokens: 1000,
            content: make_big_json(),
            critical: "FINAL_MARKER",
            query: None,
            is_json: true,
            is_diff: false,
        },
        Fixture {
            name: "large git diff (change in last file)",
            budget_chars: 2400,
            budget_tokens: 600,
            content: make_big_diff("CRITICAL_CHANGE in the last file"),
            critical: "CRITICAL_CHANGE",
            query: None,
            is_json: false,
            is_diff: true,
        },
    ];

    println!("ctx-bench — RFC-003 — BASELINE (head-truncation) vs COMPRESSED (line-importance)\n");
    println!(
        "{:<37} {:>7} | {:>8} {:>9} | {:>8} {:>9}",
        "fixture", "raw tok", "base tok", "base kept", "comp tok", "comp kept"
    );
    println!("{}", "-".repeat(92));

    let (mut base_lost, mut comp_lost) = (0, 0);
    for f in &fixtures {
        let base = head_truncate(&f.content, f.budget_chars);
        // Route through the unified content-keyed pipeline, exactly like the core.rs seams.
        let comp = if f.query.is_none() && !f.is_json && !f.is_diff {
            compress(&f.content, Kind::BuildLog, f.budget_tokens)
        } else {
            compress(&f.content, Kind::File { query: f.query, path: None }, f.budget_tokens)
        };
        let base_ok = base.contains(f.critical);
        let comp_ok = comp.contains(f.critical);
        if !base_ok {
            base_lost += 1;
        }
        if !comp_ok {
            comp_lost += 1;
        }
        println!(
            "{:<37} {:>7} | {:>8} {:>9} | {:>8} {:>9}",
            f.name,
            estimate_tokens(&f.content),
            estimate_tokens(&base),
            if base_ok { "YES" } else { "NO<-LOST" },
            estimate_tokens(&comp),
            if comp_ok { "YES" } else { "NO<-LOST" },
        );
    }

    println!("\n(tok = freecode-compress::estimate_tokens, Unicode scalars / 3.6)");
    println!(
        "BASELINE: {}/{} lose the critical line.   COMPRESSED: {}/{} lose it.",
        base_lost,
        fixtures.len(),
        comp_lost,
        fixtures.len()
    );
    if comp_lost == 0 && base_lost > 0 {
        println!("WIN: line-importance keeps every critical line the head-truncation dropped, same budget.");
    }
}
