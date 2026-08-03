//! freecode-trajectory — RFC-005 Slice 0: a read-only importer that normalizes coding-agent
//! session transcripts into a unified `Trajectory` schema, so `freecode learn` (and SLM training)
//! can mine REAL trajectories from the tools already in use instead of waiting for fresh data.
//!
//! Today it parses **Claude Code** JSONL transcripts (`~/.claude/projects/<proj>/<session>.jsonl`):
//! ordered events of type `user` / `assistant` carrying prompts, `tool_use` calls, and
//! `tool_result`s (with `is_error`). A later slice adds the VSCode/Copilot `chatSessions/*.json`
//! shape (`requests[]` of message/response/result).
//!
//! Privacy: this only READS what you point it at — the caller chooses which project dirs to import
//! (the corpus may include work you are not free to mine — keep it local and allowlisted).

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// A single tool invocation and whether its result was an error.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Action {
    pub tool: String,
    pub ok: bool,
}

/// One normalized session trajectory: the asks, the actions taken (with outcomes), and a verdict.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Trajectory {
    pub source: &'static str, // "claude-code"
    pub session_id: String,
    pub project: String,
    pub branch: String,
    pub prompts: Vec<String>, // user asks (text)
    pub actions: Vec<Action>, // tool calls in order, paired with their result outcome
    pub tool_calls: usize,     // count of tool_use seen
    pub errors: usize,         // count of error tool_results
    pub assistant_chars: usize,
    pub outcome: &'static str, // "ok" | "had-errors" | "empty"
}

/// Parse one Claude Code JSONL transcript into a `Trajectory`. Malformed lines are skipped.
/// First pass: map EVERY tool_use id → tool name across the whole session. Claude Code interleaves
/// sidechain (subagent) events, so a tool_result can appear in line order BEFORE its tool_use —
/// an incremental map would mis-attribute it to "?". Building the full map up front fixes that
/// (measured: orphan tool_results drop from 389 → ~0 over the personal corpus).
fn collect_tool_names(jsonl: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for line in jsonl.lines() {
        let Ok(ev) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if ev.get("type").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        if let Some(Value::Array(arr)) = ev.pointer("/message/content") {
            for c in arr {
                if c.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                    if let Some(id) = c.get("id").and_then(|v| v.as_str()) {
                        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        m.insert(id.to_string(), name.to_string());
                    }
                }
            }
        }
    }
    m
}

pub fn parse_claude_session(jsonl: &str, project: &str) -> Trajectory {
    let mut t = Trajectory {
        source: "claude-code",
        project: project.to_string(),
        outcome: "empty",
        ..Default::default()
    };
    // tool_use id → name (full session, sidechain-safe) so every tool_result attributes correctly.
    let id_to_name = collect_tool_names(jsonl);

    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let ev: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if t.session_id.is_empty() {
            if let Some(s) = ev.get("sessionId").and_then(|v| v.as_str()) {
                t.session_id = s.to_string();
            }
        }
        if t.branch.is_empty() {
            if let Some(b) = ev.get("gitBranch").and_then(|v| v.as_str()) {
                t.branch = b.to_string();
            }
        }
        match ev.get("type").and_then(|v| v.as_str()) {
            Some("user") => match ev.pointer("/message/content") {
                Some(Value::String(s)) => t.prompts.push(s.clone()),
                Some(Value::Array(arr)) => {
                    for c in arr {
                        match c.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                if let Some(s) = c.get("text").and_then(|v| v.as_str()) {
                                    t.prompts.push(s.to_string());
                                }
                            }
                            Some("tool_result") => {
                                let is_err = c.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                                let id = c.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                                let tool = id_to_name.get(id).cloned().unwrap_or_else(|| "?".to_string());
                                if is_err {
                                    t.errors += 1;
                                }
                                t.actions.push(Action { tool, ok: !is_err });
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            Some("assistant") => {
                if let Some(Value::Array(arr)) = ev.pointer("/message/content") {
                    for c in arr {
                        match c.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                if let Some(s) = c.get("text").and_then(|v| v.as_str()) {
                                    t.assistant_chars += s.chars().count();
                                }
                            }
                            Some("tool_use") => {
                                t.tool_calls += 1; // names resolved in the first pass (sidechain-safe)
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    t.outcome = if t.tool_calls == 0 && t.prompts.is_empty() {
        "empty"
    } else if t.errors > 0 {
        "had-errors"
    } else {
        "ok"
    };
    t
}

/// A message in a normalized SFT-style conversation (role: user | assistant | tool).
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
}

/// A full session as an SFT-ready conversation (truncated + secret-redacted) for SLM training.
#[derive(Serialize, Clone, Debug, Default)]
pub struct Conversation {
    pub source: &'static str,
    pub project: String,
    pub session_id: String,
    pub outcome: &'static str,
    /// "agentic" = one human instruction → autonomous tool chain (the corpus's 90%);
    /// "conversational" = >=2 real human turns (rare, high-value for chat-style SFT).
    pub mode: &'static str,
    /// RFC-004 task class of the opening prompt (codegen / audit / output-distill / …), via the
    /// SAME `freecode-classify` the daemon's router uses — so dataset labels match live routing.
    pub task_class: &'static str,
    pub action_count: usize,
    /// Real human prompts after injected-noise filtering — the signal `mode` is derived from.
    pub user_turns: usize,
    pub messages: Vec<Message>,
}

/// Delimiters that bound a token so an embedded secret (`API_KEY=ghp_…`, `"token":"sk-…"`) is split
/// out and caught — NOT `-`/`_`/`/` (those are part of tokens/paths, and splitting on `-` would make
/// `Flask-1` look like an `sk-` key).
const SECRET_DELIMS: &str = "=:\"'(),;|[]{}<>`*~ \t\n\r";

/// Best-effort secret redaction: whole `-----BEGIN…-----END…-----` key blocks, plus tokens that
/// start with a known key/PAT prefix or are long opaque blobs — even when embedded after `=`/`:`/
/// quotes. NOT exhaustive; the real guard is still choosing which project dirs to export.
pub fn redact(s: &str) -> String {
    let s = redact_key_blocks(s);
    s.split_inclusive(|c: char| SECRET_DELIMS.contains(c))
        .map(|piece| {
            let core_len = piece.trim_end_matches(|c: char| SECRET_DELIMS.contains(c)).len();
            let (core, delim) = piece.split_at(core_len);
            if looks_secret(core) {
                format!("[REDACTED]{delim}")
            } else {
                piece.to_string()
            }
        })
        .collect()
}

/// Replace each PEM-style `-----BEGIN … -----END … -----` block (private keys, certs) with a marker.
fn redact_key_blocks(s: &str) -> String {
    if !s.contains("-----BEGIN") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("-----BEGIN") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let block_end = after
            .find("-----END")
            .and_then(|e| after[e + 8..].find("-----").map(|c| e + 8 + c + 5));
        match block_end {
            Some(end) => {
                out.push_str("[REDACTED-KEY]");
                rest = &after[end..];
            }
            None => {
                out.push_str("[REDACTED-KEY]");
                rest = &after["-----BEGIN".len()..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn looks_secret(t: &str) -> bool {
    t.starts_with("sk-")
        || t.starts_with("ghp_")
        || t.starts_with("gho_")
        || t.starts_with("ghs_")
        || t.starts_with("ghr_")
        || t.starts_with("github_pat_")
        || t.starts_with("glpat-")
        || t.starts_with("xox")
        || t.starts_with("AKIA")
        || t.starts_with("ASIA")
        || (t.len() >= 32 && t.chars().all(|c| c.is_ascii_hexdigit()))
        || (t.len() >= 40 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
}

fn truncate_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…[truncated]")
    }
}

/// True when a `type:user` text block is NOT a real human prompt but content Claude Code injects
/// into the user channel: slash-command expansions, local-command stdout, system reminders, the
/// "[Request interrupted]" marker, the `Caveat:` local-command preamble, and the giant
/// compaction/continuation summary. Training on these as `user` turns teaches the model to emit
/// session-summaries and tooling noise as if instructed — so they're dropped (tool_results stay).
fn is_injected_user_text(s: &str) -> bool {
    let t = s.trim_start();
    t.is_empty()
        || t.starts_with("<command-name>")
        || t.starts_with("<command-message>")
        || t.starts_with("<command-args>")
        || t.starts_with("<local-command-stdout>")
        || t.starts_with("<local-command-stderr>")
        || t.starts_with("<bash-input>")
        || t.starts_with("<bash-stdout>")
        || t.starts_with("<bash-stderr>")
        || t.starts_with("<system-reminder>")
        || t.starts_with("[Request interrupted")
        || t.starts_with("Caveat:")
        || t.starts_with("This session is being continued from a previous conversation")
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    }
}

/// Parse a Claude Code session into an SFT-ready conversation: user asks, assistant text + tool
/// calls (name + args), and tool results — each truncated to `max_chars` and secret-redacted.
pub fn parse_claude_conversation(jsonl: &str, project: &str, max_chars: usize) -> Conversation {
    let mut conv = Conversation {
        source: "claude-code",
        project: project.to_string(),
        outcome: "empty",
        ..Default::default()
    };
    let id_to_name = collect_tool_names(jsonl); // full session, sidechain-safe attribution
    let mut errors = 0usize;
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let ev: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if conv.session_id.is_empty() {
            if let Some(s) = ev.get("sessionId").and_then(|v| v.as_str()) {
                conv.session_id = s.to_string();
            }
        }
        match ev.get("type").and_then(|v| v.as_str()) {
            Some("user") => {
                let is_meta = ev.get("isMeta").and_then(|v| v.as_bool()).unwrap_or(false);
                match ev.pointer("/message/content") {
                Some(Value::String(s)) => {
                    if !is_meta && !is_injected_user_text(s) {
                        conv.messages.push(Message {
                            role: "user".into(),
                            content: redact(&truncate_text(s, max_chars)),
                            ..Default::default()
                        });
                    }
                }
                Some(Value::Array(arr)) => {
                    for c in arr {
                        match c.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                if let Some(s) = c.get("text").and_then(|v| v.as_str()) {
                                    if !is_meta && !is_injected_user_text(s) {
                                        conv.messages.push(Message {
                                            role: "user".into(),
                                            content: redact(&truncate_text(s, max_chars)),
                                            ..Default::default()
                                        });
                                    }
                                }
                            }
                            Some("tool_result") => {
                                let is_err = c.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                                if is_err {
                                    errors += 1;
                                }
                                let id = c.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                                let tool = id_to_name.get(id).cloned();
                                let body = c.get("content").map(value_text).unwrap_or_default();
                                conv.messages.push(Message {
                                    role: "tool".into(),
                                    content: redact(&truncate_text(&body, max_chars)),
                                    tool,
                                    ok: Some(!is_err),
                                });
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
                }
            },
            Some("assistant") => {
                if let Some(Value::Array(arr)) = ev.pointer("/message/content") {
                    for c in arr {
                        match c.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                if let Some(s) = c.get("text").and_then(|v| v.as_str()) {
                                    if !s.trim().is_empty() {
                                        conv.messages.push(Message {
                                            role: "assistant".into(),
                                            content: redact(&truncate_text(s, max_chars)),
                                            ..Default::default()
                                        });
                                    }
                                }
                            }
                            Some("tool_use") => {
                                conv.action_count += 1;
                                let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                let args = c.get("input").map(|v| truncate_text(&v.to_string(), max_chars)).unwrap_or_default();
                                conv.messages.push(Message {
                                    role: "assistant".into(),
                                    content: redact(&args),
                                    tool: Some(name),
                                    ..Default::default()
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    conv.user_turns = conv.messages.iter().filter(|m| m.role == "user").count();
    conv.mode = if conv.user_turns >= 2 { "conversational" } else { "agentic" };
    let first_prompt = conv.messages.iter().find(|m| m.role == "user").map(|m| m.content.as_str()).unwrap_or("");
    conv.task_class = freecode_classify::classify_task(first_prompt, "auto").as_str();
    conv.outcome = if conv.action_count == 0 && conv.messages.is_empty() {
        "empty"
    } else if errors > 0 {
        "had-errors"
    } else {
        "ok"
    };
    conv
}

/// RFC-006 PIC-5 — a Battle-1 training pair: a structural `before → after` AST transformation plus
/// the local intent that drove it. Mined from Claude Code `Edit`/`MultiEdit` calls — the NARROW task
/// freelm is meant to learn ("given AST context + intent, produce the tree edit"), distinct from
/// free-form generation (`Write`) which is Battle-2 territory and intentionally excluded.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct EditPair {
    pub source: &'static str,
    pub project: String,
    pub session_id: String,
    /// Short path (last 2 components) — no absolute/home leak.
    pub file: String,
    pub lang: &'static str,
    /// The assistant's reasoning local to this edit (redacted, truncated) — the "intent".
    pub intent: String,
    /// old_string, kept FULL (truncating a transformation corrupts it) but secret-redacted.
    pub before: String,
    /// new_string (redacted). Empty = a deletion, which is a valid transformation.
    pub after: String,
}

/// First pass: tool_use ids whose tool_result was an error — so a failed Edit (old_string not found,
/// etc.) is never mined as a "good" transformation.
fn collect_errored_tool_ids(jsonl: &str) -> std::collections::HashSet<String> {
    let mut errored = std::collections::HashSet::new();
    for line in jsonl.lines() {
        let Ok(ev) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if ev.get("type").and_then(|v| v.as_str()) != Some("user") {
            continue;
        }
        if let Some(Value::Array(arr)) = ev.pointer("/message/content") {
            for c in arr {
                if c.get("type").and_then(|v| v.as_str()) == Some("tool_result")
                    && c.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false)
                {
                    if let Some(id) = c.get("tool_use_id").and_then(|v| v.as_str()) {
                        errored.insert(id.to_string());
                    }
                }
            }
        }
    }
    errored
}

fn first_user_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(arr) => arr.iter().find_map(|c| {
            if c.get("type").and_then(|v| v.as_str()) == Some("text") {
                c.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())
            } else {
                None
            }
        }),
        _ => None,
    }
}

/// (old_string, new_string) pairs from an Edit (single) or MultiEdit (edits[]) tool input.
fn extract_edit_strings(input: &Value) -> Vec<(String, String)> {
    if let Some(edits) = input.get("edits").and_then(|v| v.as_array()) {
        edits
            .iter()
            .filter_map(|e| {
                Some((
                    e.get("old_string").and_then(|v| v.as_str())?.to_string(),
                    e.get("new_string").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                ))
            })
            .collect()
    } else if let Some(o) = input.get("old_string").and_then(|v| v.as_str()) {
        vec![(o.to_string(), input.get("new_string").and_then(|v| v.as_str()).unwrap_or("").to_string())]
    } else {
        vec![]
    }
}

fn lang_from_path(p: &str) -> &'static str {
    match p.rsplit('.').next().unwrap_or("") {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "rb" => "ruby",
        "php" => "php",
        "cs" => "csharp",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "scala" => "scala",
        "sh" | "bash" | "zsh" => "shell",
        "html" => "html",
        "css" | "scss" => "css",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" => "markdown",
        "sql" => "sql",
        _ => "other",
    }
}

/// Last two path components, so the dataset carries `src/foo.rs`, not an absolute home path.
fn short_path(p: &str) -> String {
    let mut parts: Vec<&str> = p.trim_end_matches('/').rsplit('/').take(2).collect();
    parts.reverse();
    parts.join("/")
}

/// Mine Battle-1 edit pairs from a Claude Code session. Keeps only edits that (a) APPLIED cleanly
/// (their tool_result was not an error), (b) actually change something (before != after, non-empty
/// before), and (c) are LOCAL (each side <= max_chars — bigger is a rewrite, not a structural edit).
/// before/after are secret-redacted but kept FULL; intent = the assistant's reasoning local to the edit.
pub fn parse_claude_edit_pairs(jsonl: &str, project: &str, max_chars: usize) -> Vec<EditPair> {
    let errored = collect_errored_tool_ids(jsonl);
    let mut pairs = Vec::new();
    let mut session_id = String::new();
    let mut first_prompt: Option<String> = None;
    let mut last_text = String::new();
    for line in jsonl.lines() {
        let Ok(ev) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if session_id.is_empty() {
            if let Some(s) = ev.get("sessionId").and_then(|v| v.as_str()) {
                session_id = s.to_string();
            }
        }
        match ev.get("type").and_then(|v| v.as_str()) {
            Some("user") => {
                if first_prompt.is_none() {
                    if let Some(s) = ev.pointer("/message/content").and_then(first_user_text) {
                        if !is_injected_user_text(&s) {
                            first_prompt = Some(s);
                        }
                    }
                }
            }
            Some("assistant") => {
                if let Some(Value::Array(arr)) = ev.pointer("/message/content") {
                    for c in arr {
                        match c.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                if let Some(s) = c.get("text").and_then(|v| v.as_str()) {
                                    if !s.trim().is_empty() {
                                        last_text = s.to_string();
                                    }
                                }
                            }
                            Some("tool_use") => {
                                let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                if name != "Edit" && name != "MultiEdit" {
                                    continue;
                                }
                                if let Some(id) = c.get("id").and_then(|v| v.as_str()) {
                                    if errored.contains(id) {
                                        continue;
                                    }
                                }
                                let Some(input) = c.get("input") else { continue };
                                let file = input.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
                                let lang = lang_from_path(file);
                                let intent_src = if !last_text.trim().is_empty() {
                                    last_text.as_str()
                                } else {
                                    first_prompt.as_deref().unwrap_or("")
                                };
                                let intent = redact(&truncate_text(intent_src, max_chars));
                                for (before, after) in extract_edit_strings(input) {
                                    if before.is_empty() || before == after {
                                        continue;
                                    }
                                    if before.chars().count() > max_chars || after.chars().count() > max_chars {
                                        continue;
                                    }
                                    let (rb, ra) = (redact(&before), redact(&after));
                                    // A secret-only change collapses to a no-op after redaction → drop it
                                    // (useless to train on, and a faint signal a secret was edited).
                                    if rb == ra {
                                        continue;
                                    }
                                    pairs.push(EditPair {
                                        source: "claude-code",
                                        project: project.to_string(),
                                        session_id: session_id.clone(),
                                        file: short_path(file),
                                        lang,
                                        intent: intent.clone(),
                                        before: rb,
                                        after: ra,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_pairs_mined_skipping_failed_and_degenerate() {
        let jsonl = concat!(
            r#"{"type":"user","sessionId":"S","message":{"content":"refactor the adder"}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"I'll switch unwrap to ?"},{"type":"tool_use","id":"e1","name":"Edit","input":{"file_path":"/home/dev/projects/demo/src/foo.rs","old_string":"let x = a.unwrap();","new_string":"let x = a?;"}}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"e1","is_error":false,"content":"ok"}]}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"e2","name":"Edit","input":{"file_path":"/x/y/b.rs","old_string":"nope","new_string":"nada"}}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"e2","is_error":true,"content":"old_string not found"}]}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"e3","name":"Edit","input":{"file_path":"/x/y/c.rs","old_string":"same","new_string":"same"}}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"e3","is_error":false,"content":"ok"}]}}"#, "\n"
        );
        let pairs = parse_claude_edit_pairs(jsonl, "demo", 2000);
        assert_eq!(pairs.len(), 1, "only the clean, non-degenerate, applied edit survives");
        let p = &pairs[0];
        assert_eq!(p.before, "let x = a.unwrap();");
        assert_eq!(p.after, "let x = a?;");
        assert_eq!(p.lang, "rust");
        assert_eq!(p.file, "src/foo.rs");
        assert!(p.intent.contains("unwrap to ?"), "intent = the assistant's local reasoning");
    }

    #[test]
    fn tool_result_before_its_tool_use_still_attributes_sidechain_safe() {
        // Sidechain (subagent) interleaving can place a tool_result AHEAD of its tool_use in line
        // order; the two-pass parse must still name it, not fall back to "?".
        let jsonl = concat!(
            r#"{"type":"user","sessionId":"S","message":{"content":[{"type":"tool_result","tool_use_id":"x9","is_error":false,"content":"ok"}]}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"x9","name":"Grep","input":{"pattern":"foo"}}]}}"#, "\n"
        );
        let t = parse_claude_session(jsonl, "freecode");
        assert_eq!(t.actions.len(), 1);
        assert_eq!(t.actions[0].tool, "Grep", "must resolve via the first pass, not '?'");
    }

    #[test]
    fn parses_a_minimal_trajectory() {
        // user ask → assistant tool_use(Bash) → user tool_result(ok)
        let jsonl = concat!(
            r#"{"type":"user","sessionId":"S1","gitBranch":"main","message":{"content":[{"type":"text","text":"run the tests"}]}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":false}]}}"#, "\n"
        );
        let t = parse_claude_session(jsonl, "freecode");
        assert_eq!(t.session_id, "S1");
        assert_eq!(t.branch, "main");
        assert_eq!(t.project, "freecode");
        assert_eq!(t.prompts, vec!["run the tests".to_string()]);
        assert_eq!(t.tool_calls, 1);
        assert_eq!(t.actions, vec![Action { tool: "Bash".into(), ok: true }]);
        assert_eq!(t.errors, 0);
        assert_eq!(t.outcome, "ok");
    }

    #[test]
    fn flags_error_outcome_and_attributes_tool() {
        let jsonl = concat!(
            r#"{"type":"assistant","sessionId":"S2","message":{"content":[{"type":"tool_use","id":"x","name":"Edit","input":{}}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"x","is_error":true}]}}"#, "\n"
        );
        let t = parse_claude_session(jsonl, "p");
        assert_eq!(t.errors, 1);
        assert_eq!(t.actions, vec![Action { tool: "Edit".into(), ok: false }]);
        assert_eq!(t.outcome, "had-errors");
    }

    #[test]
    fn skips_malformed_lines() {
        let t = parse_claude_session("not json\n\n{bad}\n", "p");
        assert_eq!(t.outcome, "empty");
    }

    #[test]
    fn conversation_redacts_secrets_and_keeps_structure() {
        let jsonl = concat!(
            r#"{"type":"user","sessionId":"S","message":{"content":[{"type":"text","text":"deploy with ghp_abcdefghijklmnopqrstuvwxyz0123456789"}]}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":false,"content":"ok"}]}}"#, "\n"
        );
        let conv = parse_claude_conversation(jsonl, "freecode", 10_000);
        assert_eq!(conv.outcome, "ok");
        assert_eq!(conv.action_count, 1);
        assert_eq!(conv.mode, "agentic", "single human turn → agentic");
        assert!(conv.messages.iter().any(|m| m.content.contains("[REDACTED]")), "secret redacted");
        assert!(!conv.messages.iter().any(|m| m.content.contains("ghp_abc")), "raw token must be gone");
        let tool_msg = conv.messages.iter().find(|m| m.role == "tool").unwrap();
        assert_eq!(tool_msg.tool.as_deref(), Some("Bash"));
        assert_eq!(tool_msg.ok, Some(true));
    }

    #[test]
    fn redact_leaves_normal_text_alone() {
        assert_eq!(redact("just refactor the auth module please"), "just refactor the auth module please");
    }

    #[test]
    fn injected_user_noise_is_dropped_real_prompts_kept() {
        let jsonl = concat!(
            r#"{"type":"user","sessionId":"S","message":{"content":"fix the parser bug"}}"#, "\n",
            r#"{"type":"user","isMeta":true,"message":{"content":[{"type":"text","text":"<command-name>/cost</command-name>"}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#, "\n",
            r#"{"type":"user","message":{"content":"This session is being continued from a previous conversation that ran out of context. Summary: ..."}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Edit","input":{"x":1}}]}}"#, "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":false,"content":"done"}]}}"#, "\n",
            r#"{"type":"user","message":{"content":"now run the tests"}}"#, "\n"
        );
        let conv = parse_claude_conversation(jsonl, "freecode", 10_000);
        let users: Vec<&str> = conv.messages.iter().filter(|m| m.role == "user").map(|m| m.content.as_str()).collect();
        assert_eq!(users, vec!["fix the parser bug", "now run the tests"], "only the 2 real prompts survive");
        assert_eq!(conv.user_turns, 2);
        assert_eq!(conv.mode, "conversational", "2 real human turns → conversational");
        // tool_result still captured (filter must not touch the tool channel)
        assert!(conv.messages.iter().any(|m| m.role == "tool" && m.tool.as_deref() == Some("Edit")));
    }

    #[test]
    fn redact_catches_embedded_secrets_not_just_whitespace_tokens() {
        // The bug the whitespace-only splitter missed: secrets glued to a key=/":"/quote.
        assert_eq!(redact("export API_KEY=ghp_abcdefghijklmnopqrstuvwxyz0123456789"), "export API_KEY=[REDACTED]");
        assert!(!redact(r#"{"token":"sk-proj-abcdefghijklmnopqrstuvwxyz123456"}"#).contains("sk-proj"));
        // ...but a normal hyphenated word that merely contains "sk-" is left alone (no false positive).
        assert_eq!(redact("Flask-3 and a Task-7 ticket"), "Flask-3 and a Task-7 ticket");
        // Whole PEM key block collapses to a marker.
        let pem = "head -----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n-----END RSA PRIVATE KEY----- tail";
        let r = redact(pem);
        assert!(r.contains("[REDACTED-KEY]") && !r.contains("MIIEowIBAAKCAQEA") && r.contains("tail"));
    }
}
