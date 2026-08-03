//! Content-keyed compression pipeline (RFC-003 §3.1) — the single entry point that routes a piece
//! of context to the right deterministic compressor, replacing the if-chain at the daemon seams.
//!
//! The call site states the KIND (it knows whether it's holding a build log or a file the model
//! read — freecode's tools are typed). For a file read, the pipeline detects the structure
//! (unified diff → JSON → source) and runs the matching compressor; a structure-aware compressor
//! that leaves the result over budget (e.g. a JSON object with several array fields) falls through
//! to line-importance [`fit`]. Build logs go straight to [`compress_log`].

use crate::{compress_diff, compress_json, compress_log, estimate_tokens, fit, Ctx};

/// What kind of context is being compressed. Known at the call site.
pub enum Kind<'a> {
    /// Compiler / build / test output → log-aware compression (dedup warnings, keep errors).
    BuildLog,
    /// A file the model read. `query` = the turn's question (relevance signal for source files);
    /// `path` aids structure detection by extension. Content auto-classified: diff → JSON → source.
    File { query: Option<&'a str>, path: Option<&'a str> },
}

/// Route `text` to the right compressor for its `kind` and fit it to `budget_tokens`.
pub fn compress(text: &str, kind: Kind, budget_tokens: usize) -> String {
    match kind {
        Kind::BuildLog => compress_log(text, budget_tokens),
        Kind::File { query, path } => {
            let head = text.trim_start();
            let ext = |suf: &str| path.is_some_and(|p| p.ends_with(suf));
            if ext(".diff") || ext(".patch") || head.starts_with("diff --git") || text.contains("\n@@ ") {
                // No fit fall-through here: fit is line-importance, not diff-aware, and could drop
                // the +/- change lines. compress_diff already trims to a (soft) budget while keeping
                // hunk structure + the change intact — a slightly-over-budget intact diff beats a
                // budget-exact shredded one.
                compress_diff(text, budget_tokens)
            } else if ext(".json") || matches!(head.bytes().next(), Some(b'[') | Some(b'{')) {
                fall_through(compress_json(text, budget_tokens), query, budget_tokens)
            } else {
                fit(text, budget_tokens, Ctx::Source { query })
            }
        }
    }
}

/// A structure-aware pass can leave content over budget; finish with line-importance `fit`.
fn fall_through(staged: String, query: Option<&str>, budget_tokens: usize) -> String {
    if estimate_tokens(&staged) > budget_tokens {
        fit(&staged, budget_tokens, Ctx::Source { query })
    } else {
        staged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_build_log() {
        let mut t = String::new();
        for i in 0..400 {
            t.push_str(&format!("warning: unused variable: `tmp{}`\n", i));
        }
        t.push_str("error[E0599]: boom\n");
        let out = compress(&t, Kind::BuildLog, 400);
        assert!(out.contains("error[E0599]"), "log path must keep the error");
        assert!(out.lines().count() <= 4, "log path must collapse the duplicate warnings: {out}");
    }

    #[test]
    fn routes_json_by_content() {
        let mut s = String::from("[");
        for i in 0..500 {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(r#"{{"id":{},"tail":"END_{}"}}"#, i, i));
        }
        s.push(']');
        let out = compress(&s, Kind::File { query: None, path: None }, 300);
        assert!(out.contains("items elided"), "json path: array sampled");
        assert!(out.contains("END_499"), "json path: tail kept");
    }

    #[test]
    fn routes_diff_by_extension() {
        let mut t = String::from("");
        for f in 0..30 {
            t.push_str(&format!("@@ -1,3 +1,3 @@ ctx{}\n ctx\n-old{}\n+new{}\n", f, f, f));
        }
        let out = compress(&t, Kind::File { query: None, path: Some("x.patch") }, 200);
        assert!(estimate_tokens(&out) < estimate_tokens(&t), "diff path: compressed");
    }

    #[test]
    fn routes_source_to_fit() {
        let mut t = String::new();
        for i in 0..800 {
            t.push_str(&format!("fn helper_{}() {{}}\n", i));
        }
        t.push_str("pub fn targetSymbol() {}\n");
        let out = compress(&t, Kind::File { query: Some("targetSymbol"), path: Some("m.rs") }, 1000);
        assert!(out.contains("targetSymbol"), "source path: query-relevant symbol kept by fit");
    }
}
