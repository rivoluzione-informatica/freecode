//! freecode-compress — deterministic, dependency-free context compression (RFC-003).
//!
//! Replaces blind HEAD-truncation with line-importance + query-relevance selection so the
//! lines the model actually needs (the rustc error, the asked-about symbol) survive the
//! budget instead of being cut off the tail.
//!
//! Ported *in spirit* (not code) from headroom-core/signals (Apache-2.0): the keyword sets
//! and their documented bugfixes — `token` dropped from the security set (false-positives on
//! "LLM token"), and `abort/timeout/denied/rejected` added to the error set.
//!
//! One external dependency, deliberately: the build-log seam feeds l0-compressor's filters
//! (`compress_log` → `denoise`) instead of reimplementing noise removal here. Everything else —
//! token estimation, line scoring, budget fitting, JSON/diff seams — stays local and
//! dependency-free.

use std::collections::HashSet;

pub mod adaptive;
pub mod diff;
pub mod json;
pub mod log;
pub mod pipeline;
pub use diff::compress_diff;
pub use json::compress_json;
pub use log::compress_log;
pub use pipeline::{compress, Kind};

pub(crate) const ERROR: &[&str] = &[
    "error", "exception", "fail", "failed", "failure", "fatal", "critical", "crash", "panic",
    "abort", "timeout", "denied", "rejected",
];
const SECURITY: &[&str] = &["security", "auth", "password", "secret"]; // `token` intentionally dropped
pub(crate) const WARNING: &[&str] = &["warn", "warning"];
const IMPORTANCE: &[&str] = &["important", "note", "todo", "fixme", "hack", "xxx", "bug", "fix"];

const P_QUERY: f32 = 0.97;
const P_ERROR: f32 = 0.95;
const P_SECURITY: f32 = 0.85;
const P_WARNING: f32 = 0.75;
const P_SIGNATURE: f32 = 0.70;
const P_IMPORTANCE: f32 = 0.60;
/// Lines scoring at/above this are NEVER dropped (RFC-003 §5 invariant).
pub const KEEP_FLOOR: f32 = 0.85;

#[derive(Clone, Copy)]
pub enum Ctx<'a> {
    /// Build / compiler / log output: keyword-importance (error keywords are objective).
    Log,
    /// Source file read: code signatures + optional query relevance.
    Source { query: Option<&'a str> },
}

/// Rough token estimate over Unicode **scalars** (RFC-003 W2). cpt ≈ 3.6.
/// Counts characters, not bytes, so multi-byte source files stop over-counting.
pub fn estimate_tokens(s: &str) -> usize {
    let chars = s.chars().count() as f32;
    (((chars / 3.6) + 0.5) as usize).max(1)
}

pub(crate) fn words(line: &str) -> HashSet<String> {
    line.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

pub(crate) fn any_in(set: &[&str], ws: &HashSet<String>) -> bool {
    set.iter().any(|k| ws.contains(*k))
}

/// Word-set Jaccard similarity in [0,1] over lowercased alphanumeric tokens (RFC-003 W3 dedup).
/// 1.0 = identical word sets, 0.0 = disjoint. Two empty notes count as identical.
pub fn jaccard_words(a: &str, b: &str) -> f32 {
    let wa = words(a);
    let wb = words(b);
    if wa.is_empty() && wb.is_empty() {
        return 1.0;
    }
    let union = wa.union(&wb).count();
    if union == 0 {
        return 0.0;
    }
    wa.intersection(&wb).count() as f32 / union as f32
}

fn is_signature(line: &str) -> bool {
    let t = line.trim_start();
    const SIGS: &[&str] = &[
        "fn ", "pub fn ", "pub async fn ", "async fn ", "struct ", "pub struct ", "enum ",
        "trait ", "impl ", "class ", "def ", "export ", "function ", "type ", "interface ",
    ];
    SIGS.iter().any(|s| t.starts_with(s))
}

/// Importance score in [0,1] for a single line under `ctx`.
pub fn score_line(line: &str, ctx: Ctx) -> f32 {
    let ws = words(line);
    let mut s = 0.0f32;
    if any_in(ERROR, &ws) {
        s = s.max(P_ERROR);
    }
    if any_in(SECURITY, &ws) {
        s = s.max(P_SECURITY);
    }
    if any_in(WARNING, &ws) {
        s = s.max(P_WARNING);
    }
    if any_in(IMPORTANCE, &ws) {
        s = s.max(P_IMPORTANCE);
    }
    if let Ctx::Source { query } = ctx {
        if is_signature(line) {
            s = s.max(P_SIGNATURE);
        }
        if let Some(q) = query {
            let qw = words(q);
            // query relevance: a non-trivial query word (len>=4) appears on the line.
            if qw.iter().any(|w| w.len() >= 4 && ws.contains(w)) {
                s = s.max(P_QUERY);
            }
        }
    }
    s
}

/// Fit `text` into ~`budget_tokens` (estimated, RFC-003 W2), keeping the highest-importance lines (plus a ±1 context
/// window) in original order. Lines scoring >= [`KEEP_FLOOR`] are ALWAYS kept (the invariant:
/// never drop the error / query-matched line). Under budget → returned unchanged.
pub fn fit(text: &str, budget_tokens: usize, ctx: Ctx) -> String {
    if estimate_tokens(text) <= budget_tokens {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    if n == 0 {
        return text.to_string();
    }
    let scores: Vec<f32> = lines.iter().map(|l| score_line(l, ctx)).collect();
    let cost = |i: usize| estimate_tokens(lines[i]); // token cost of the line (RFC-003 W2)

    // deterministic selection order: score desc, then original index asc.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        scores[b]
            .partial_cmp(&scores[a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });

    // Each emitted "…[N lines elided]…" marker costs tokens too; charge it so a fragmented
    // keep-set can't silently overflow the budget (RFC-003 Slice-2 fit polish).
    const MARKER_TOKENS: i64 = 6;
    let budget = budget_tokens as i64;
    // Number of dropped runs (= elision markers) in the current keep[] — edges are boundaries.
    let marker_count = |keep: &[bool]| -> i64 {
        let mut runs = 0i64;
        let mut j = 0;
        while j < n {
            if keep[j] {
                j += 1;
            } else {
                runs += 1;
                while j < n && !keep[j] {
                    j += 1;
                }
            }
        }
        runs
    };
    // Change in marker count from keeping currently-dropped line `i`.
    let marker_delta = |keep: &[bool], i: usize| -> i64 {
        let left_b = i == 0 || keep[i - 1];
        let right_b = i + 1 == n || keep[i + 1];
        match (left_b, right_b) {
            (true, true) => -1,  // a singleton dropped run disappears
            (false, false) => 1, // splits one dropped run into two
            _ => 0,              // merely shrinks a run at its edge
        }
    };

    let mut keep = vec![false; n];

    // pass 1 — INVARIANT: force-keep every >= floor line + a ±1 context window (no budget gate).
    for &i in &order {
        if scores[i] >= KEEP_FLOOR && !keep[i] {
            keep[i] = true;
            if i > 0 {
                keep[i - 1] = true;
            }
            if i + 1 < n {
                keep[i + 1] = true;
            }
        }
    }

    // budget already consumed by the invariant set: its lines plus the markers it implies.
    let mut used: i64 =
        (0..n).filter(|&i| keep[i]).map(|i| cost(i) as i64).sum::<i64>() + marker_count(&keep) * MARKER_TOKENS;

    // pass 2 — fill remaining budget by descending score, charging each line's marker delta.
    for &i in &order {
        if keep[i] {
            continue;
        }
        let marginal = cost(i) as i64 + marker_delta(&keep, i) * MARKER_TOKENS;
        if used + marginal > budget {
            continue;
        }
        keep[i] = true;
        used += marginal;
    }
    // pass 3 — keep the first line or two (schema/context) if there is room.
    for i in 0..n.min(2) {
        if !keep[i] {
            let marginal = cost(i) as i64 + marker_delta(&keep, i) * MARKER_TOKENS;
            if used + marginal <= budget {
                keep[i] = true;
                used += marginal;
            }
        }
    }

    // emit in original order; collapse dropped runs into an elision marker.
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        if keep[i] {
            out.push_str(lines[i]);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_budget_returns_unchanged() {
        let t = "alpha\nbeta\ngamma\n";
        assert_eq!(fit(t, 10_000, Ctx::Log), t);
    }

    #[test]
    fn fragmented_keep_set_respects_budget() {
        // Mid-score signatures (0.70 < KEEP_FLOOR) interleaved with low-score comments, no
        // invariant lines. Before the marker-budget fix the elision markers blew the budget far
        // past target (the ctx-bench source fixture overflowed to ~9.8k tok at a 6k budget).
        let mut t = String::new();
        for i in 0..600 {
            t.push_str(&format!("// filler comment number {}\n", i));
            t.push_str(&format!("fn helper_{}() {{}}\n", i));
        }
        let budget = 800;
        let out = fit(&t, budget, Ctx::Source { query: None });
        let got = estimate_tokens(&out);
        assert!(got <= budget + 50, "output {got} tok must respect the {budget}-tok budget");
        assert!(got < estimate_tokens(&t), "must compress");
    }

    #[test]
    fn keeps_error_line_past_budget() {
        // the exact failure RFC-003 targets: error after a wall of warnings.
        let mut t = String::new();
        for i in 0..400 {
            t.push_str(&format!("warning: unused variable `x{}`\n", i));
        }
        t.push_str("error[E0599]: no method named `frobnicate` found for struct `Widget`\n");
        let out = fit(&t, 2000, Ctx::Log);
        assert!(out.contains("error[E0599]"), "error line MUST survive truncation");
        assert!(out.chars().count() < t.chars().count(), "must actually compress");
    }

    #[test]
    fn keeps_query_matched_symbol() {
        let mut t = String::new();
        // 800 lines so the fixture clears the token budget and actually exercises selection.
        for i in 0..800 {
            t.push_str(&format!("fn helper_{}() {{}}\n", i));
        }
        t.push_str("pub fn targetSymbol() {}\n");
        let out = fit(&t, 2000, Ctx::Source { query: Some("where is targetSymbol defined") });
        assert!(out.contains("targetSymbol"), "query-matched symbol MUST survive");
    }

    #[test]
    fn token_estimate_counts_scalars() {
        assert_eq!(estimate_tokens(""), 1);
        assert!(estimate_tokens(&"a".repeat(360)) >= 90);
    }

    #[test]
    fn token_estimate_unicode_not_bytes() {
        // 100 multi-byte chars: byte len ~300, char len 100 → ~28 tokens, not ~83.
        let s = "é".repeat(100);
        assert!(estimate_tokens(&s) < 40, "must count scalars, not bytes");
    }

    #[test]
    fn jaccard_word_set_similarity() {
        assert!(jaccard_words("the fix is in core.rs", "the fix is in core.rs") > 0.99); // identical
        assert!(jaccard_words("alpha beta gamma delta", "alpha beta gamma epsilon") >= 0.5); // 3/5
        assert!(jaccard_words("completely different words here", "nothing in common at all") < 0.2);
        assert_eq!(jaccard_words("", ""), 1.0);
    }
}
