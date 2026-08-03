//! Build/test-log compressor for cargo/npm/pytest/jest/make-style output. Two passes, two
//! owners:
//!
//! 1. **Noise** — [`denoise`] runs l0-compressor's filters: ANSI stripping, runs of identical or
//!    near-identical lines collapsed, blank-line pile-ups squeezed, unchanged diff context
//!    folded. That work used to be reimplemented here; it is l0-compressor's whole job, it has
//!    73 tests behind it, and consuming it means a filtering bug found through FreeCode is a bug
//!    fixed for the CLI too.
//! 2. **Selection** — this module classifies what remains (error / warning / stack-trace /
//!    summary), dedups warnings with a prefix-preserving normalizer, and fits the result into a
//!    token budget. Errors are NEVER dropped (RFC-003 §5 invariant), same as [`crate::fit`].
//!
//! The order is load-bearing: removing redundancy first means the budget is spent on distinct
//! content instead of on 400 copies of one warning. Frequently pass 1 alone gets under budget,
//! and nothing is dropped at all.
//!
//! Selection logic ported *in spirit* from headroom-core `transforms/log_compressor.rs`
//! (Apache-2.0), including its prefix-preserving dedup fix: normalize only the region AFTER the
//! first `:`/`=`, so two distinct errors sharing a trailing address shape stay distinct.

use std::collections::HashMap;

use crate::adaptive::compute_optimal_k;
use crate::estimate_tokens;

/// Cap on stack-trace lines kept across the whole log (headroom's `stack_trace_max_lines`).
const MAX_STACK_LINES: usize = 40;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Error,
    Warn,
    Info,
}

/// Strip the redundancy out of a log with l0-compressor's filters, WITHOUT letting it truncate.
///
/// Division of labour: l0-compressor decides what is *noise* (ANSI escapes, runs of identical or
/// near-identical lines, blank-line pile-ups, unchanged diff context); this module decides what
/// *fits* and what must survive regardless. Keeping both jobs in one place is how the two
/// implementations drifted apart in the first place.
///
/// The caps are the safety-critical part. `HeadTailBuffer` drops the MIDDLE of a stream once its
/// head fills — which would silently delete an error buried in a long log and break the RFC-003
/// §5 invariant that errors are never dropped. Passing `usize::MAX` (the "raw mode" the buffer
/// documents) disables truncation entirely, so this pass only ever removes redundancy: every
/// distinct line survives into the budget-driven selection below.
fn denoise(text: &str) -> String {
    use l0_compressor::filter::{strip_ansi, FilterPipeline};
    use std::borrow::Cow;

    let cleaned = strip_ansi(text.as_bytes());
    let mut pipeline = FilterPipeline::new(usize::MAX, 0, false);
    for line in cleaned.lines() {
        pipeline.feed(Cow::Borrowed(line));
    }
    // threshold = usize::MAX: render everything the buffer holds, no head/tail elision.
    pipeline.finish(usize::MAX, text.len(), 0).output
}

/// Compress build/test-log `text` into roughly `budget_tokens`, keeping the lines a model needs
/// to act (errors + context, deduped warnings, stack traces, summaries). Under budget → unchanged.
pub fn compress_log(text: &str, budget_tokens: usize) -> String {
    if estimate_tokens(text) <= budget_tokens {
        return text.to_string();
    }
    // Noise first (l0-compressor), selection second (here). Doing it in this order means the
    // budget is spent on distinct content instead of on 400 copies of the same warning.
    let denoised = denoise(text);
    let text: &str = if estimate_tokens(&denoised) <= budget_tokens {
        // Removing redundancy alone got us under budget — nothing needs to be dropped at all.
        return denoised;
    } else {
        &denoised
    };
    let raw: Vec<&str> = text.lines().collect();
    let n = raw.len();
    if n == 0 {
        return text.to_string();
    }

    // ── classify every line ──────────────────────────────────────────────
    let mut level = vec![Level::Info; n];
    let mut is_stack = vec![false; n];
    let mut is_summary = vec![false; n];
    let mut score = vec![0.0f32; n];
    let mut prev_stack = false;
    for i in 0..n {
        let c = raw[i];
        level[i] = classify_level(c);
        let cont = prev_stack && (c.starts_with(' ') || c.starts_with('\t'));
        is_stack[i] = is_stack_opener(c) || cont;
        prev_stack = is_stack[i] && !c.trim().is_empty();
        is_summary[i] = is_summary_line(c);
        score[i] = score_log_line(level[i], is_stack[i], is_summary[i]);
    }

    // ── select ───────────────────────────────────────────────────────────
    let mut keep = vec![false; n];
    let mut dup = vec![1usize; n]; // dup[i] > 1 ⇒ line i stands in for collapsed duplicates

    let mut stack_kept = 0usize;
    for i in 0..n {
        if level[i] == Level::Error {
            keep[i] = true; // errors + ±1 context, always
            if i > 0 {
                keep[i - 1] = true;
            }
            if i + 1 < n {
                keep[i + 1] = true;
            }
        }
        if is_summary[i] {
            keep[i] = true;
        }
        if is_stack[i] && stack_kept < MAX_STACK_LINES {
            keep[i] = true;
            stack_kept += 1;
        }
    }

    // warnings: prefix-preserving dedup — keep the first of each normalized key, count the rest.
    let mut warn_first: HashMap<String, usize> = HashMap::new();
    for i in 0..n {
        if level[i] == Level::Warn {
            let key = normalize_for_dedupe(raw[i]);
            match warn_first.get(&key) {
                None => {
                    warn_first.insert(key, i);
                    keep[i] = true;
                }
                Some(&first) => dup[first] += 1,
            }
        }
    }

    let cost = |i: usize| estimate_tokens(raw[i]) + if dup[i] > 1 { 4 } else { 0 };

    // adaptive line cap: a content-aware sanity bound over the kept set (never drops errors/summaries).
    let kept_contents: Vec<&str> = (0..n).filter(|&i| keep[i]).map(|i| raw[i]).collect();
    let cap = compute_optimal_k(&kept_contents, 1.0, 10, None);
    enforce_cap(&mut keep, &score, &level, &is_summary, cap);

    // hard token budget: drop the lowest-scoring non-error kept lines until we fit.
    let mut used: usize = (0..n).filter(|&i| keep[i]).map(&cost).sum();
    if used > budget_tokens {
        let mut order: Vec<usize> = (0..n).filter(|&i| keep[i]).collect();
        order.sort_by(|&a, &b| {
            score[a].partial_cmp(&score[b]).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b))
        });
        for &i in &order {
            if used <= budget_tokens {
                break;
            }
            if level[i] != Level::Error {
                keep[i] = false;
                used = used.saturating_sub(cost(i));
            }
        }
    }

    // ── emit in original order; collapse dropped runs into an elision marker ─
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        if keep[i] {
            out.push_str(raw[i]);
            if dup[i] > 1 {
                out.push_str(&format!("  (×{} similar)", dup[i]));
            }
            out.push('\n');
            i += 1;
        } else {
            let start = i;
            while i < n && !keep[i] {
                i += 1;
            }
            out.push_str(&format!("…[{} lines elided]…\n", i - start));
        }
    }
    out
}

fn enforce_cap(keep: &mut [bool], score: &[f32], level: &[Level], is_summary: &[bool], cap: usize) {
    let kept: Vec<usize> = (0..keep.len()).filter(|&i| keep[i]).collect();
    if kept.len() <= cap {
        return;
    }
    let mut order = kept.clone();
    order.sort_by(|&a, &b| {
        score[a].partial_cmp(&score[b]).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b))
    });
    let mut to_drop = kept.len() - cap;
    for &i in &order {
        if to_drop == 0 {
            break;
        }
        if level[i] != Level::Error && !is_summary[i] {
            keep[i] = false;
            to_drop -= 1;
        }
    }
}

fn classify_level(content: &str) -> Level {
    let ws = crate::words(content);
    if crate::any_in(crate::ERROR, &ws) {
        Level::Error
    } else if crate::any_in(crate::WARNING, &ws) {
        Level::Warn
    } else {
        Level::Info
    }
}

fn score_log_line(level: Level, is_stack: bool, is_summary: bool) -> f32 {
    let base: f32 = match level {
        Level::Error => 1.0,
        Level::Warn => 0.5,
        Level::Info => 0.1,
    };
    let stack: f32 = if is_stack { 0.3 } else { 0.0 };
    let summary: f32 = if is_summary { 0.4 } else { 0.0 };
    (base + stack + summary).min(1.0_f32)
}

/// Opener of a stack-trace, across Python / JS / Java / Rust / Go flavors (headroom's
/// `StackTraceDetector::flavor_for`, condensed).
fn is_stack_opener(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("Traceback (most recent call last)") {
        return true;
    }
    if t.starts_with("File \"") && t.contains("\", line ") {
        return true; // Python frame
    }
    if t.starts_with("--> ") && has_line_col(t) {
        return true; // Rust `--> file:line:col`
    }
    if t.starts_with("at ") && (t.contains('(') || has_line_col(t)) {
        return true; // JS / Java frame
    }
    false
}

/// True if `s` contains a `:<digits>:<digits>` (line:col) marker.
fn has_line_col(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i] == b':' && b[i + 1].is_ascii_digit() {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j < b.len() && b[j] == b':' && b.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Summary/footer lines worth keeping verbatim (pytest/jest separators, cargo `test result:`,
/// `N passed/failed`, `Tests: …`, `TOTAL`/`Summary`).
fn is_summary_line(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("===") || t.starts_with("---") {
        return true;
    }
    if t.starts_with("test result:") || t.starts_with("error: aborting") {
        return true;
    }
    let lower = t.to_lowercase();
    if t.chars().next().is_some_and(|c| c.is_ascii_digit())
        && ["passed", "failed", "skipped", "error", "warning"].iter().any(|k| lower.contains(k))
    {
        return true;
    }
    if ["tests:", "test:", "suites:", "suite:"].iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    t.starts_with("TOTAL") || t.starts_with("Total") || t.starts_with("Summary")
}

/// Prefix-preserving dedup key: keep everything before the first `:`/`=` verbatim, mask only the
/// trailing variable region (hex addresses → `ADDR`, digit runs → `N`, `/path/like/` → `/PATH/`).
fn normalize_for_dedupe(content: &str) -> String {
    let split_at = content.find([':', '=']).unwrap_or(content.len());
    let (prefix, suffix) = content.split_at(split_at);
    format!("{}{}", prefix, mask_variable_region(suffix))
}

fn mask_variable_region(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < n {
        let c = chars[i];
        // hex address: 0x<hex+>
        if c == '0' && i + 2 < n && (chars[i + 1] == 'x' || chars[i + 1] == 'X') && chars[i + 2].is_ascii_hexdigit() {
            let mut j = i + 2;
            while j < n && chars[j].is_ascii_hexdigit() {
                j += 1;
            }
            out.push_str("ADDR");
            i = j;
            continue;
        }
        // digit run
        if c.is_ascii_digit() {
            while i < n && chars[i].is_ascii_digit() {
                i += 1;
            }
            out.push('N');
            continue;
        }
        // `/path/like/` — a slash, path chars, ending in a slash
        if c == '/' {
            let mut j = i + 1;
            while j < n && (chars[j] == '/' || chars[j].is_alphanumeric() || matches!(chars[j], '_' | '.' | '-')) {
                j += 1;
            }
            if j > i + 1 && chars[j - 1] == '/' {
                out.push_str("/PATH/");
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_budget_unchanged() {
        let t = "warning: a\nerror: boom\n";
        assert_eq!(compress_log(t, 10_000), t);
    }

    #[test]
    fn dedups_warnings_and_keeps_error() {
        // 400 near-identical warnings (differ only in an index) then the real error.
        let mut t = String::new();
        for i in 0..400 {
            t.push_str(&format!("warning: unused variable: `tmp{}`\n", i));
        }
        t.push_str("error[E0599]: no method named `frobnicate` found for `Widget`\n");
        let out = compress_log(&t, 400);
        assert!(out.contains("error[E0599]"), "error MUST survive");
        // Assert the OUTCOME, not whose marker did it: the redundancy collapses (l0-compressor's
        // fuzzy signature matching now catches these before the budget pass ever sees them) and
        // the result is a couple of lines instead of 400.
        assert!(out.lines().count() <= 4, "400 near-identical warnings must collapse: {out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&t) / 4, "must compress hard");
    }

    /// The one thing the l0-compressor pass could have broken. Its `HeadTailBuffer` drops the
    /// MIDDLE of a stream once the head fills, so an error buried in a long log would vanish
    /// silently — `denoise` disables truncation precisely to prevent that. Pin it.
    #[test]
    fn denoise_never_drops_an_error_buried_in_the_middle() {
        let mut t = String::new();
        for i in 0..5000 {
            t.push_str(&format!("info: step {i} completed\n"));
        }
        t.push_str("error[E0425]: cannot find value `needle_in_the_middle`\n");
        for i in 0..5000 {
            t.push_str(&format!("info: step {i} continued\n"));
        }
        let denoised = denoise(&t);
        assert!(
            denoised.contains("needle_in_the_middle"),
            "denoise truncated the middle and lost an error"
        );
        let out = compress_log(&t, 200);
        assert!(out.contains("needle_in_the_middle"), "error lost through the full pipeline");
    }

    /// ANSI colour codes now get stripped before anything else looks at the text — a coloured
    /// `cargo` log used to spend budget on escape sequences.
    #[test]
    fn ansi_escapes_are_stripped_before_budgeting() {
        let t = "\u{1b}[1m\u{1b}[31merror[E0308]\u{1b}[0m: mismatched types\n".repeat(200);
        let out = compress_log(&t, 50);
        assert!(!out.contains('\u{1b}'), "ANSI escapes survived into the model's context");
        assert!(out.contains("E0308"), "the error itself must survive");
    }

    #[test]
    fn distinct_errors_not_collapsed_by_dedupe() {
        // The normalizer masks only the region AFTER the first `:`/`=`; the prefix is verbatim.
        // Same prefix + differing trailing address/line → same key (dedups).
        let a = normalize_for_dedupe("warning: unused var at 0xdeadbeef line 12");
        let b = normalize_for_dedupe("warning: unused var at 0xcafef00d line 99");
        assert_eq!(a, b, "addresses/digits in the suffix masked → same key");
        // Different message prefix → distinct key (segfault and heap-overflow don't merge).
        let c = normalize_for_dedupe("error: heap overflow at 0xdeadbeef line 12");
        assert_ne!(a, c, "different message prefix → distinct key");
    }
}
