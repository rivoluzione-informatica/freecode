//! T1 edit-generator path (RFC-006 tier T1): a small **local** model proposes a
//! SEARCH→REPLACE edit, the existing deterministic hard gates validate it, and
//! [`freecode_verdict::route`] decides Ship / RetrySameTier / EscalateToT2.
//!
//! The model never decides — the gates do. The T1 model (a fused Qwen3-1.7B LoRA)
//! is served OpenAI-compatibly at a raw `/v1/completions` endpoint and trained on
//! the exact tag format built by [`build_patch_prompt`]. It is deliberately narrow
//! (produce one local edit, given the `before` span); on a miss (no applicable
//! edit) or a gate veto the turn escalates to T2 (the main model + full agent loop).
//!
//! This module is additive and side-effect-free: it composes the *real* gates
//! ([`crate::safety_gate`], [`crate::core::run_compile_check`]/`run_test_check`)
//! and the verdict firewall. The production `dispatch_intent` path is untouched;
//! the daemon hook (behind a default-OFF flag + an IDE-selection field) is the
//! next slice.

use crate::safety_gate::{scan_content, worst_severity, Severity};
use freecode_verdict::{route, HardVerdict, Route};
use serde::{Deserialize, Serialize};

/// Separates the search span from the replace span. MUST byte-match the token the
/// model was trained to emit.
const SEP: &str = "<|sep|>";
/// Chat-control tokens different model families terminate on. We trim any of them
/// off the completion before parsing (an instruct model may emit one as a stop).
const CHAT_STOPS: &[&str] = &["<|im_end|>", "<|endoftext|>", "<|eot_id|>", "<|end_of_text|>"];

/// Build the exact raw-completion prompt the T1 model was fine-tuned on. The model
/// continues from the trailing `<|patch|>` with `search<|sep|>replace`.
pub fn build_patch_prompt(intent: &str, lang: &str, file: &str, before: &str) -> String {
    format!(
        "<|intent|>{intent}<|/intent|><|lang|>{lang}<|/lang|>\
         <|file|>{file}<|/file|><|before|>{before}<|/before|><|patch|>"
    )
}

// ---------------------------------------------------------------------------
// Raw /v1/completions client. llm.rs only carries the chat shape; the T1 model
// is trained on RAW completion (no chat template), so it needs this endpoint.
// ---------------------------------------------------------------------------
#[derive(Serialize)]
struct CompletionRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    max_tokens: u32,
    temperature: f32,
    stop: Vec<&'a str>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    text: String,
}

#[derive(Deserialize)]
struct CompletionResponse {
    choices: Vec<CompletionChoice>,
}

/// POST a raw completion (greedy, temperature 0) and return the generated text.
pub async fn complete(endpoint: &str, model: &str, prompt: &str, max_tokens: u32) -> Result<String, String> {
    let req = CompletionRequest {
        model,
        prompt,
        max_tokens,
        temperature: 0.0,
        stop: CHAT_STOPS.to_vec(),
    };
    // Non-streamed → a hard total timeout is the right bound (audit P1.4).
    let client = crate::llm::blocking_client();
    let resp = client
        .post(endpoint)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("t1 request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("t1 endpoint returned {}", resp.status()));
    }
    let body: CompletionResponse = resp.json().await.map_err(|e| format!("t1 decode failed: {e}"))?;
    body.choices
        .into_iter()
        .next()
        .map(|c| c.text)
        .ok_or_else(|| "t1: empty choices".to_string())
}

/// Split a completion into `(search, replace)`. Trims any chat-control tail, then
/// splits on the first `<|sep|>`. None when no separator or an empty search (an
/// empty search is unlocatable → not an applicable edit).
pub fn parse_patch(gen: &str) -> Option<(String, String)> {
    let mut g = gen;
    for s in CHAT_STOPS {
        if let Some(i) = g.find(s) {
            g = &g[..i];
        }
    }
    let i = g.find(SEP)?;
    let search = g[..i].to_string();
    let replace = g[i + SEP.len()..].to_string();
    if search.is_empty() {
        return None;
    }
    Some((search, replace))
}

/// Apply a SEARCH→REPLACE to `before` (first occurrence). None if `search` is
/// empty or not found — i.e. the edit does not cleanly apply.
pub fn apply_patch(before: &str, search: &str, replace: &str) -> Option<String> {
    if search.is_empty() {
        return None;
    }
    let i = before.find(search)?;
    let mut out = String::with_capacity(before.len() - search.len() + replace.len());
    out.push_str(&before[..i]);
    out.push_str(replace);
    out.push_str(&before[i + search.len()..]);
    Some(out)
}

/// A T1-proposed edit that cleanly applied to `before`.
pub struct ProposedEdit {
    pub rel_path: String,
    pub old_text: String,
    pub new_text: String,
    /// `before` with the patch applied — the content the safety gate scans.
    pub after: String,
}

/// One T1 attempt: prompt → endpoint → parse → apply. None means the model did not
/// produce an applicable edit (the caller escalates to T2).
pub async fn propose_edit(
    endpoint: &str,
    model: &str,
    intent: &str,
    lang: &str,
    rel_path: &str,
    before: &str,
) -> Option<ProposedEdit> {
    let prompt = build_patch_prompt(intent, lang, rel_path, before);
    let gen = complete(endpoint, model, &prompt, 256).await.ok()?;
    let (search, replace) = parse_patch(&gen)?;
    let after = apply_patch(before, &search, &replace)?;
    Some(ProposedEdit {
        rel_path: rel_path.to_string(),
        old_text: search,
        new_text: replace,
        after,
    })
}

/// Run the deterministic hard gates on a proposed edit and return the verdict list.
/// `project` enables the compile + test gates (they need a real workspace); when
/// None, only the safety/structural gate runs (content-only). The caller feeds the
/// result to [`freecode_verdict::route`].
pub fn validate(
    rel_path: &str,
    introduced_content: &str,
    project: Option<&crate::core::ProjectCheck>,
) -> Vec<HardVerdict> {
    let mut verdicts = Vec::new();

    // Slop & Safety gate — Error findings (secrets, merge markers, hidden chars) veto.
    let findings = scan_content(rel_path, introduced_content);
    match worst_severity(&findings) {
        Some(Severity::Error) => {
            let why = findings
                .iter()
                .filter(|f| f.severity == Severity::Error)
                .map(|f| format!("{}: {}", f.rule, f.message))
                .collect::<Vec<_>>()
                .join("; ");
            verdicts.push(HardVerdict::Veto(format!("safety_gate: {why}")));
        }
        _ => verdicts.push(HardVerdict::Pass),
    }

    // Compile + test gates — only when a real project is supplied. Ok(None) = passed
    // or no build/tests (no veto); Ok(Some(err)) = hard veto; Err = gate unavailable
    // (emit no verdict rather than a spurious pass/veto).
    if let Some(p) = project {
        match crate::core::run_compile_check(p) {
            Ok(Some(err)) => verdicts.push(HardVerdict::Veto(format!("compile: {}", first_line(&err)))),
            Ok(None) => verdicts.push(HardVerdict::Pass),
            Err(_) => {}
        }
        match crate::core::run_test_check(p) {
            Ok(Some(err)) => verdicts.push(HardVerdict::Veto(format!("tests: {}", first_line(&err)))),
            Ok(None) => verdicts.push(HardVerdict::Pass),
            Err(_) => {}
        }
    }

    verdicts
}

/// Pick the most informative line of a compiler/test failure for the veto reason
/// (this is what T2 sees on escalation): prefer the real `error[..]`/`FAILED` line
/// over cargo's "Checking …" progress noise; fall back to the first non-empty line.
fn first_line(s: &str) -> String {
    s.lines()
        .find(|l| {
            let t = l.trim_start();
            t.starts_with("error") || t.contains("error[") || t.contains("FAILED") || t.contains("panicked")
        })
        .or_else(|| s.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

/// Full T1 decision: validate the proposed edit through the gates and route it.
/// `Ship` → apply; `RetrySameTier`/`EscalateToT2` → hand off (with the veto reasons).
pub fn decide(
    edit: &ProposedEdit,
    project: Option<&crate::core::ProjectCheck>,
    retries_used: usize,
    max_retries: usize,
) -> (Route, Vec<HardVerdict>) {
    let verdicts = validate(&edit.rel_path, &edit.after, project);
    let r = route(&verdicts, retries_used, max_retries);
    (r, verdicts)
}

/// Outcome of a T1 fast-path turn (the daemon hook).
pub enum T1Decision {
    /// The edit passed the gates and is applied on disk.
    Shipped { rel_path: String, after: String, gates_passed: usize },
    /// No applicable edit, or a gate vetoed (file restored) → the daemon runs T2.
    Escalate(String),
}

/// Map a file path to the model's training `lang` value (best effort).
pub fn lang_from_path(path: &str) -> &'static str {
    match std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("") {
        "rs" => "rust",
        "py" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" => "javascript",
        "go" => "go",
        "rb" => "ruby",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" | "cxx" => "cpp",
        "sh" | "bash" => "shell",
        _ => "other",
    }
}

/// Splice the post-edit region into the file by replacing the (UNIQUE) IDE selection
/// span. None if the selection is empty, missing (stale), or non-unique — in those
/// cases we will not guess where to edit and escalate instead.
fn splice_selection(file_content: &str, selection: &str, after: &str) -> Option<String> {
    if selection.is_empty() {
        return None;
    }
    let first = file_content.find(selection)?;
    if file_content[first + selection.len()..].contains(selection) {
        return None; // ambiguous: appears more than once
    }
    let mut out = String::with_capacity(file_content.len() - selection.len() + after.len());
    out.push_str(&file_content[..first]);
    out.push_str(after);
    out.push_str(&file_content[first + selection.len()..]);
    Some(out)
}

/// The T1 fast-path the daemon calls: propose → apply to disk → validate via the REAL
/// gates → route. On Ship the edit stays applied; on a miss or veto the file is
/// restored to its original content and the turn escalates to T2. The ONLY disk write
/// is to the selected file; an escalation always leaves it byte-for-byte unchanged.
/// Resolve the served model id. If `model` is set, use it; otherwise query the
/// endpoint's /v1/models and pick the fused T1 model (mlx_lm.server lists ALL cached
/// models, so prefer the "fused" one over data[0]).
async fn resolve_model(endpoint: &str, model: &str) -> String {
    if !model.is_empty() {
        return model.to_string();
    }
    let url = endpoint.replace("/v1/completions", "/v1/models");
    if let Ok(r) = reqwest::get(&url).await {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            if let Some(arr) = v["data"].as_array() {
                if let Some(id) = arr
                    .iter()
                    .filter_map(|m| m["id"].as_str())
                    .find(|id| id.contains("fused") || id.contains("qwen3-t1"))
                    .or_else(|| arr.first().and_then(|m| m["id"].as_str()))
                {
                    return id.to_string();
                }
            }
        }
    }
    "default".to_string()
}

pub async fn try_fastpath(
    endpoint: &str,
    model: &str,
    intent: &str,
    workspace: &str,
    rel_file: &str,
    selection: &str,
) -> T1Decision {
    let lang = lang_from_path(rel_file);
    let model = resolve_model(endpoint, model).await;
    let edit = match propose_edit(endpoint, &model, intent, lang, rel_file, selection).await {
        Some(e) => e,
        None => return T1Decision::Escalate("T1 produced no applicable edit".into()),
    };
    let path = match crate::core::resolve_in_workspace(workspace, rel_file) {
        Ok(p) => p,
        Err(e) => return T1Decision::Escalate(format!("path: {e}")),
    };
    let original = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return T1Decision::Escalate(format!("read {rel_file}: {e}")),
    };
    let new_content = match splice_selection(&original, selection, &edit.after) {
        Some(c) => c,
        None => return T1Decision::Escalate("selection not uniquely found in file (stale?)".into()),
    };
    // A no-op edit (the model echoed the selection) is valid + compiles but does nothing
    // useful — the gates can't catch it, so don't ship a do-nothing turn; escalate to T2.
    if new_content == original {
        return T1Decision::Escalate("T1 edit was a no-op (model echoed the selection)".into());
    }
    if let Err(e) = std::fs::write(&path, &new_content) {
        return T1Decision::Escalate(format!("write {rel_file}: {e}"));
    }
    // Validate through the REAL gates (compile/test read the now-written file).
    let project = path
        .parent()
        .and_then(|d| crate::core::detect_project(d, std::path::Path::new(workspace)));
    let verdicts = validate(rel_file, &edit.after, project.as_ref());
    // One-shot tier: retries exhausted (1/1) → any veto routes straight past Ship.
    if matches!(route(&verdicts, 1, 1), Route::Ship) {
        T1Decision::Shipped {
            rel_path: rel_file.to_string(),
            after: edit.after,
            gates_passed: verdicts.len(),
        }
    } else {
        let _ = std::fs::write(&path, &original); // revert before escalating to T2
        T1Decision::Escalate(format!(
            "T1 edit vetoed by gates: {}",
            freecode_verdict::veto_reasons(&verdicts).join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_replace() {
        let (s, r) = parse_patch("mod a;<|sep|>mod a;\npub mod b;").unwrap();
        assert_eq!(s, "mod a;");
        assert_eq!(r, "mod a;\npub mod b;");
    }

    #[test]
    fn trims_chat_control_tail_before_parsing() {
        let (s, r) = parse_patch("x = 1<|sep|>x = 2<|im_end|>\ngarbage after").unwrap();
        assert_eq!(s, "x = 1");
        assert_eq!(r, "x = 2");
    }

    #[test]
    fn no_sep_or_empty_search_is_not_an_edit() {
        assert!(parse_patch("no separator here").is_none());
        assert!(parse_patch("<|sep|>only replace").is_none()); // empty search
    }

    #[test]
    fn applies_unique_patch() {
        assert_eq!(apply_patch("a b c", "b", "B"), Some("a B c".to_string()));
        assert_eq!(apply_patch("a b c", "zzz", "B"), None); // not found
        assert_eq!(apply_patch("a b c", "", "B"), None); // empty search
    }

    #[test]
    fn splices_unique_selection_only() {
        let file = "fn a() {}\nfn target() { 1 }\nfn b() {}\n";
        assert_eq!(
            splice_selection(file, "fn target() { 1 }", "fn target() { 2 }"),
            Some("fn a() {}\nfn target() { 2 }\nfn b() {}\n".to_string())
        );
        assert!(splice_selection(file, "absent", "x").is_none()); // stale selection
        assert!(splice_selection("x x", "x", "y").is_none()); // ambiguous (non-unique)
        assert!(splice_selection(file, "", "y").is_none()); // empty
    }

    #[test]
    fn safety_gate_vetoes_a_secret_edit_but_passes_clean() {
        // A clean edit ships.
        let clean = validate("src/lib.rs", "pub fn add(a: i32, b: i32) -> i32 { a + b }", None);
        assert_eq!(route(&clean, 0, 3), Route::Ship);

        // An edit that introduces a hardcoded AWS key is vetoed and (retries left) retries.
        let secret = validate("src/lib.rs", "let k = \"AKIAIOSFODNN7EXAMPLE1\";", None);
        assert!(secret.iter().any(HardVerdict::is_veto));
        assert_eq!(route(&secret, 0, 3), Route::RetrySameTier);
        assert_eq!(route(&secret, 3, 3), Route::EscalateToT2); // exhausted → escalate
    }

    // -----------------------------------------------------------------------
    // LIVE measurement (opt-in, ignored by default so `cargo test` stays hermetic):
    //   1. fuse + serve the T1 model:  python -m mlx_lm fuse ... ; mlx_lm server --model qwen3-t1-fused --port 7999
    //   2. FREECODE_T1_ENDPOINT=http://127.0.0.1:7999/v1/completions \
    //      FREECODE_T1_MODEL=qwen3-t1-fused \
    //      FREECODE_T1_DATA=/path/to/your/eval-set.jsonl \
    //      FREECODE_T1_N=40 \
    //      cargo test --bin freecode-daemon t1_shiprate_live -- --ignored --nocapture
    // Reports, over held-out Battle-1 cases: T1 applied% (vs escalate-on-miss) and
    // of the applied edits, how many pass the safety gate → Ship vs Veto. The
    // compile/test gates that discriminate correct-vs-broken need a live repo edit
    // (the daemon hook = next slice); they are wired in `validate(..)` already.
    // -----------------------------------------------------------------------
    #[derive(Deserialize)]
    struct Row {
        intent: String,
        lang: String,
        file: String,
        before: String,
    }

    #[tokio::test]
    #[ignore]
    async fn t1_shiprate_live() {
        let endpoint = std::env::var("FREECODE_T1_ENDPOINT")
            .unwrap_or_else(|_| "http://127.0.0.1:7999/v1/completions".into());
        // Auto-detect the served model id (mlx_lm.server derives it from --model).
        let models_url = endpoint.replace("/v1/completions", "/v1/models");
        // mlx_lm.server lists ALL cached models; pick the fused T1 one, not data[0].
        let model = match reqwest::get(&models_url).await {
            Ok(r) => r
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| {
                    let arr = v["data"].as_array()?.clone();
                    arr.iter()
                        .filter_map(|m| m["id"].as_str())
                        .find(|id| id.contains("fused") || id.contains("qwen3-t1"))
                        .or_else(|| arr.first().and_then(|m| m["id"].as_str()))
                        .map(String::from)
                })
                .unwrap_or_else(|| "default".into()),
            Err(_) => "default".into(),
        };
        println!("[t1] endpoint={endpoint} served-model={model}");
        let data = std::env::var("FREECODE_T1_DATA").expect("set FREECODE_T1_DATA=path/to/test.jsonl");
        let n: usize = std::env::var("FREECODE_T1_N").ok().and_then(|v| v.parse().ok()).unwrap_or(40);

        let text = std::fs::read_to_string(&data).expect("read data");
        let rows: Vec<Row> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .take(n)
            .collect();
        assert!(!rows.is_empty(), "no rows parsed from {data}");

        let (mut applied, mut ship, mut escalate, total) = (0usize, 0usize, 0usize, rows.len());
        for r in &rows {
            match propose_edit(&endpoint, &model, &r.intent, &r.lang, &r.file, &r.before).await {
                Some(edit) => {
                    applied += 1;
                    // project=None → safety/structural gate only (snippets aren't a buildable repo)
                    let (route_decision, _v) = decide(&edit, None, 0, 3);
                    match route_decision {
                        Route::Ship => ship += 1,
                        _ => escalate += 1, // safety veto → retry/escalate
                    }
                }
                None => escalate += 1, // T1 produced no applicable edit → straight to T2
            }
        }
        println!("\n=== T1 ship-rate via REAL gates (n={total}, endpoint={endpoint}) ===");
        println!("  T1 produced an applicable edit : {applied}/{total} = {:.0}%", 100.0 * applied as f64 / total as f64);
        println!("  → SHIP (passed safety gate)    : {ship}/{total} = {:.0}%", 100.0 * ship as f64 / total as f64);
        println!("  → escalate to T2 (miss or veto): {escalate}/{total} = {:.0}%", 100.0 * escalate as f64 / total as f64);
        println!("  (compile/test gates wired in validate(), need a live repo = next slice)");
    }

    // The DISCRIMINATING gate: a compiling edit Ships, a non-compiling one is vetoed
    // by the REAL compile gate and (retries exhausted) escalates to T2. Proves the
    // full stack catches BROKEN edits, not just inapplicable ones. Needs `cargo`;
    // ignored by default (it compiles a temp crate). Run:
    //   cargo test --bin freecode-daemon t1_compile_gate_demo -- --ignored --nocapture
    #[test]
    #[ignore]
    fn t1_compile_gate_demo() {
        use freecode_verdict::veto_reasons;
        use std::fs;

        let dir = std::env::temp_dir().join(format!("freecode_t1_gate_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"t1demo\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
        let project = crate::core::detect_project(&dir, &dir).expect("temp rust project detected");

        let good = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n";
        let broken = "pub fn add(a: i32, b: i32) -> i32 { a + \"oops\" }\n"; // type error → won't compile

        // GOOD edit: write to disk, run full stack (safety + compile + test), route.
        fs::write(dir.join("src/lib.rs"), good).unwrap();
        let r_good = route(&validate("src/lib.rs", good, Some(&project)), 0, 3);
        println!("\n[compile-gate demo] GOOD   edit -> {}", r_good.as_str());
        assert_eq!(r_good, Route::Ship, "a compiling edit must Ship");

        // BROKEN edit: the compile gate must VETO; route can never Ship; exhausted → escalate.
        fs::write(dir.join("src/lib.rs"), broken).unwrap();
        let v = validate("src/lib.rs", broken, Some(&project));
        println!(
            "[compile-gate demo] BROKEN edit -> route(0/3)={} route(3/3)={} veto={:?}",
            route(&v, 0, 3).as_str(),
            route(&v, 3, 3).as_str(),
            veto_reasons(&v),
        );
        assert!(v.iter().any(HardVerdict::is_veto), "broken edit must be vetoed by the compile gate");
        assert_ne!(route(&v, 0, 3), Route::Ship, "a non-compiling edit must NEVER Ship");
        assert_eq!(route(&v, 3, 3), Route::EscalateToT2, "exhausted retries → escalate to T2");

        let _ = fs::remove_dir_all(&dir);
        println!("[compile-gate demo] OK: full gate stack ships good edits, vetoes+escalates broken ones.");
    }

    // End-to-end fast-path on a REAL temp repo + the live T1 model. Proves the
    // safety-critical INVARIANT (regardless of whether the unreliable model ships):
    //   Shipped  ⇒ the file changed AND it still compiles
    //   Escalate ⇒ the file is byte-for-byte restored (no half-applied/broken state)
    // Needs the server up. Run:
    //   FREECODE_T1_ENDPOINT=http://127.0.0.1:7999/v1/completions \
    //     cargo test --bin freecode-daemon t1_fastpath_e2e -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn t1_fastpath_e2e() {
        use std::fs;
        let endpoint = std::env::var("FREECODE_T1_ENDPOINT")
            .unwrap_or_else(|_| "http://127.0.0.1:7999/v1/completions".into());

        let dir = std::env::temp_dir().join(format!("freecode_t1_e2e_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"t1e2e\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
        let selection = "    let max_retries = 3;";
        fs::write(
            dir.join("src/lib.rs"),
            format!("pub fn budget() -> i32 {{\n{selection}\n    max_retries * 10\n}}\n"),
        )
        .unwrap();
        let original = fs::read_to_string(dir.join("src/lib.rs")).unwrap();

        let decision = try_fastpath(
            &endpoint,
            "",
            "bump max_retries from 3 to 5",
            dir.to_str().unwrap(),
            "src/lib.rs",
            selection,
        )
        .await;

        let after_file = fs::read_to_string(dir.join("src/lib.rs")).unwrap();
        match decision {
            T1Decision::Shipped { rel_path, gates_passed, .. } => {
                println!("[t1 e2e] SHIPPED to {rel_path} ({gates_passed} gates)");
                assert_ne!(after_file, original, "ship must have changed the file");
                let proj = crate::core::detect_project(&dir, &dir).unwrap();
                assert!(
                    crate::core::run_compile_check(&proj).unwrap().is_none(),
                    "a shipped edit must compile"
                );
            }
            T1Decision::Escalate(reason) => {
                println!("[t1 e2e] ESCALATE: {reason}");
                assert_eq!(after_file, original, "escalate must leave the file byte-for-byte unchanged");
            }
        }
        let _ = fs::remove_dir_all(&dir);
        println!("[t1 e2e] invariant held.");
    }
}
