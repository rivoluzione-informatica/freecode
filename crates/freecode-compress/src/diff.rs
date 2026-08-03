//! Unified-diff compressor — ported *in spirit* (dependency-free) from headroom-core
//! `transforms/diff_compressor.rs` (Apache-2.0). Verbose `git diff` output is trimmed to the lines
//! a model needs: each `+`/`-` change plus ±N context lines (the rest of the context collapsed),
//! per-file hunk cap (first + last + heaviest), file cap (heaviest), then a token budget. Short
//! diffs / non-diffs / already-under-budget → returned unchanged.
//!
//! Dep-free: manual line scan rather than the regex/md5 parser. Deferred vs headroom: query-relevance
//! hunk scoring, CCR hashing, combined/`@@@` (merge) diffs.

use crate::estimate_tokens;

const MAX_CONTEXT: usize = 2;
const MAX_HUNKS_PER_FILE: usize = 10;
const MAX_FILES: usize = 20;
const MIN_LINES: usize = 50;

struct Hunk<'a> {
    header: &'a str,
    body: Vec<&'a str>,
}

struct DiffFile<'a> {
    pre: Vec<&'a str>, // "diff --git", "index", "---", "+++" header lines
    hunks: Vec<Hunk<'a>>,
}

/// Compress unified-diff `text` toward `budget_tokens`. See module docs for the strategy.
pub fn compress_diff(text: &str, budget_tokens: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < MIN_LINES || estimate_tokens(text) <= budget_tokens {
        return text.to_string();
    }
    if !lines.iter().any(|l| l.starts_with("@@")) {
        return text.to_string(); // not a unified diff — leave to other compressors
    }
    let mut files = parse(&lines);
    if files.is_empty() {
        return text.to_string();
    }

    // File cap: keep the heaviest (most changes) in original order, note the rest.
    let dropped_files = if files.len() > MAX_FILES {
        let mut idx: Vec<usize> = (0..files.len()).collect();
        idx.sort_by(|&a, &b| file_changes(&files[b]).cmp(&file_changes(&files[a])));
        let keep: std::collections::BTreeSet<usize> = idx.into_iter().take(MAX_FILES).collect();
        let mut kept = Vec::new();
        let mut dropped = 0usize;
        for (i, f) in files.into_iter().enumerate() {
            if keep.contains(&i) {
                kept.push(f);
            } else {
                dropped += 1;
            }
        }
        files = kept;
        dropped
    } else {
        0
    };

    let mut out = String::new();
    for f in &files {
        for h in &f.pre {
            out.push_str(h);
            out.push('\n');
        }
        let (kept, dropped_hunks) = select_hunks(&f.hunks);
        for h in &kept {
            out.push_str(h.header);
            out.push('\n');
            for l in reduce_context(&h.body, MAX_CONTEXT) {
                out.push_str(&l);
                out.push('\n');
            }
        }
        if dropped_hunks > 0 {
            out.push_str(&format!("  … {} more hunk(s) in this file elided …\n", dropped_hunks));
        }
    }
    if dropped_files > 0 {
        out.push_str(&format!("… {} more changed file(s) elided …\n", dropped_files));
    }

    // Final token budget: if still over, drop whole hunks lowest-change-first (keep file headers).
    if estimate_tokens(&out) > budget_tokens {
        out = budget_trim(&files, dropped_files, budget_tokens);
    }
    out
}

fn parse<'a>(lines: &[&'a str]) -> Vec<DiffFile<'a>> {
    let mut files: Vec<DiffFile<'a>> = Vec::new();
    let mut cur: Option<DiffFile<'a>> = None;
    for &l in lines {
        if l.starts_with("diff --git") {
            if let Some(f) = cur.take() {
                files.push(f);
            }
            cur = Some(DiffFile { pre: vec![l], hunks: Vec::new() });
        } else if l.starts_with("@@") {
            let f = cur.get_or_insert_with(|| DiffFile { pre: Vec::new(), hunks: Vec::new() });
            f.hunks.push(Hunk { header: l, body: Vec::new() });
        } else if let Some(f) = cur.as_mut() {
            match f.hunks.last_mut() {
                Some(h) => h.body.push(l),
                None => f.pre.push(l), // pre-hunk file header (index / --- / +++)
            }
        }
    }
    if let Some(f) = cur.take() {
        files.push(f);
    }
    files
}

fn hunk_changes(h: &Hunk) -> usize {
    h.body.iter().filter(|l| l.starts_with('+') || l.starts_with('-')).count()
}

fn file_changes(f: &DiffFile) -> usize {
    f.hunks.iter().map(hunk_changes).sum()
}

/// Keep first + last + heaviest middle hunks (capped), restored to original order. Returns the
/// kept hunk indices and the dropped count.
fn select_hunks<'a, 'b>(hunks: &'b [Hunk<'a>]) -> (Vec<&'b Hunk<'a>>, usize) {
    let n = hunks.len();
    if n <= MAX_HUNKS_PER_FILE {
        return (hunks.iter().collect(), 0);
    }
    let mut keep: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    keep.insert(0);
    keep.insert(n - 1);
    let mut middle: Vec<usize> = (1..n - 1).collect();
    middle.sort_by(|&a, &b| hunk_changes(&hunks[b]).cmp(&hunk_changes(&hunks[a])));
    for &i in middle.iter().take(MAX_HUNKS_PER_FILE.saturating_sub(2)) {
        keep.insert(i);
    }
    let kept: Vec<&Hunk> = keep.iter().map(|&i| &hunks[i]).collect();
    (kept, n - keep.len())
}

/// Keep each `+`/`-` line plus ±`max_ctx` context; collapse longer unchanged runs into a marker.
/// `\ No newline…` markers are always kept.
fn reduce_context(body: &[&str], max_ctx: usize) -> Vec<String> {
    let changes: Vec<usize> = body
        .iter()
        .enumerate()
        .filter_map(|(i, l)| (l.starts_with('+') || l.starts_with('-')).then_some(i))
        .collect();
    if changes.is_empty() {
        return body.iter().take(max_ctx).map(|s| s.to_string()).collect();
    }
    let mut keep: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for &p in &changes {
        keep.insert(p);
        for i in p.saturating_sub(max_ctx)..p {
            keep.insert(i);
        }
        for i in (p + 1)..(p + max_ctx + 1).min(body.len()) {
            keep.insert(i);
        }
    }
    for (i, l) in body.iter().enumerate() {
        if l.starts_with('\\') {
            keep.insert(i);
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        if keep.contains(&i) {
            out.push(body[i].to_string());
            i += 1;
        } else {
            let start = i;
            while i < body.len() && !keep.contains(&i) {
                i += 1;
            }
            out.push(format!("  … {} unchanged lines …", i - start));
        }
    }
    out
}

/// Last-resort budget fit: keep file headers + the heaviest hunks across all files until budget,
/// dropping the rest (lowest-change-first), each dropped hunk replaced by a one-line marker.
fn budget_trim(files: &[DiffFile], dropped_files: usize, budget_tokens: usize) -> String {
    // Rank every (file, hunk) by change count desc; keep until budget.
    let mut ranked: Vec<(usize, usize)> = Vec::new(); // (file_idx, hunk_idx)
    for (fi, f) in files.iter().enumerate() {
        for hi in 0..f.hunks.len() {
            ranked.push((fi, hi));
        }
    }
    ranked.sort_by(|&(fa, ha), &(fb, hb)| {
        hunk_changes(&files[fb].hunks[hb]).cmp(&hunk_changes(&files[fa].hunks[ha]))
    });
    let mut keep: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
    let mut used: usize = files.iter().map(|f| f.pre.iter().map(|l| estimate_tokens(l)).sum::<usize>()).sum();
    for &(fi, hi) in &ranked {
        let body = reduce_context(&files[fi].hunks[hi].body, MAX_CONTEXT);
        let cost = estimate_tokens(files[fi].hunks[hi].header)
            + body.iter().map(|l| estimate_tokens(l)).sum::<usize>();
        if !keep.is_empty() && used + cost > budget_tokens {
            continue;
        }
        used += cost;
        keep.insert((fi, hi));
    }
    let mut out = String::new();
    for (fi, f) in files.iter().enumerate() {
        for h in &f.pre {
            out.push_str(h);
            out.push('\n');
        }
        let mut elided = 0usize;
        for hi in 0..f.hunks.len() {
            if keep.contains(&(fi, hi)) {
                out.push_str(f.hunks[hi].header);
                out.push('\n');
                for l in reduce_context(&f.hunks[hi].body, MAX_CONTEXT) {
                    out.push_str(&l);
                    out.push('\n');
                }
            } else {
                elided += 1;
            }
        }
        if elided > 0 {
            out.push_str(&format!("  … {} hunk(s) elided (budget) …\n", elided));
        }
    }
    if dropped_files > 0 {
        out.push_str(&format!("… {} more changed file(s) elided …\n", dropped_files));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big_diff(critical_line: &str) -> String {
        let mut s = String::new();
        for f in 0..30 {
            s.push_str(&format!("diff --git a/file{}.rs b/file{}.rs\n", f, f));
            s.push_str(&format!("--- a/file{}.rs\n+++ b/file{}.rs\n", f, f));
            s.push_str(&format!("@@ -1,8 +1,8 @@ fn f{}()\n", f));
            for c in 0..6 {
                s.push_str(&format!(" unchanged context line {} of file {}\n", c, f));
            }
            if f == 29 {
                // The file you actually care about is the heaviest hunk — survives the caps.
                for k in 0..5 {
                    s.push_str(&format!("-old crit {}\n", k));
                }
                s.push_str(&format!("+{}\n", critical_line));
                for k in 0..4 {
                    s.push_str(&format!("+new crit {}\n", k));
                }
            } else {
                s.push_str(&format!("-old line {}\n+new line {}\n", f, f));
            }
        }
        s
    }

    #[test]
    fn short_diff_unchanged() {
        let t = "diff --git a/x b/x\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(compress_diff(t, 5), t); // < MIN_LINES
    }

    #[test]
    fn non_diff_unchanged() {
        let t = "just prose\n".repeat(60);
        assert_eq!(compress_diff(&t, 5), t); // no @@ headers
    }

    #[test]
    fn keeps_changes_drops_context_and_far_files() {
        let t = big_diff("CRITICAL_CHANGE in the last file");
        let out = compress_diff(&t, 600);
        assert!(out.contains("CRITICAL_CHANGE"), "the buried change MUST survive");
        assert!(out.contains("unchanged lines …"), "long unchanged context should be collapsed");
        assert!(estimate_tokens(&out) < estimate_tokens(&t), "must compress");
    }
}
