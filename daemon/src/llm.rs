use serde::{Deserialize, Serialize};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Egress resilience (audit P1.4). Every outbound call is bounded, and a local
// backend that accepts the socket and then goes quiet must NOT hang the turn.
//
// A *total* request timeout is wrong for the streaming path — the body only
// "finishes" when generation does, so a global cap would truncate long, healthy
// answers. The bound that actually matters there is a per-chunk stall timeout
// (`STREAM_STALL_TIMEOUT`), applied at each `next().await`. Non-streaming calls
// (T1) use `blocking_client()`, which does carry a total timeout.
// ---------------------------------------------------------------------------

/// Refuse to wait forever on a TCP handshake (backend down, or a half-open socket).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Max silence between two streamed chunks before the stream is declared dead.
pub const STREAM_STALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Total budget for a single non-streamed completion (T1 is small + greedy).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);

/// Environment variable holding the bearer token for an authenticated OpenAI-compatible backend
/// (oMLX, LiteLLM, a remote vLLM). Unset or blank means no `Authorization` header is sent, which
/// is what LM Studio and a loopback llama.cpp expect. Read from the daemon's environment only: a
/// secret does not belong in `.freecode/config.json`, which lives inside the workspace.
pub const API_KEY_ENV: &str = "FREECODE_LLM_API_KEY";

/// `Authorization: Bearer <key>` as default headers for every request a client sends, or an empty
/// map when there is no usable key. The value is marked sensitive so it never shows in debug
/// output. A key that is not a valid header value is dropped rather than panicking the daemon.
pub fn auth_headers(key: Option<&str>) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(k) = key.map(str::trim).filter(|k| !k.is_empty()) {
        if let Ok(mut v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {k}")) {
            v.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, v);
        }
    }
    headers
}

/// Path of a file holding the token, for when the daemon runs as a service and the secret should
/// not sit in a LaunchAgent plist or a systemd unit (both world-readable by default and visible in
/// `ps`/`launchctl print`). The file is read on every client build, so rotating the key needs no
/// daemon restart. `FREECODE_LLM_API_KEY` wins when both are set.
pub const API_KEY_FILE_ENV: &str = "FREECODE_LLM_API_KEY_FILE";

/// The token to use: the environment variable, else the first line of the key file, else none.
/// An unreadable file yields none (with one warning on stderr), never a panic: a daemon that dies
/// because a key file is missing is worse than one that gets a clear 401 from the backend.
pub fn resolve_api_key(env_key: Option<&str>, key_file: Option<&str>) -> Option<String> {
    if let Some(k) = env_key.map(str::trim).filter(|k| !k.is_empty()) {
        return Some(k.to_string());
    }
    let path = key_file.map(str::trim).filter(|p| !p.is_empty())?;
    match std::fs::read_to_string(path) {
        Ok(text) => {
            warn_if_group_or_world_readable(path);
            text.lines().next().map(|l| l.trim().to_string()).filter(|k| !k.is_empty())
        }
        Err(e) => {
            warn_once(&format!("[llm] cannot read {API_KEY_FILE_ENV}={path}: {e}"));
            None
        }
    }
}

fn warn_once(msg: &str) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!("{msg}");
    }
}

#[cfg(unix)]
fn warn_if_group_or_world_readable(path: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().mode() & 0o077 != 0 {
            warn_once(&format!("[llm] {path} is readable by other users; run: chmod 600 {path}"));
        }
    }
}

#[cfg(not(unix))]
fn warn_if_group_or_world_readable(_path: &str) {}

fn env_auth_headers() -> reqwest::header::HeaderMap {
    let key = resolve_api_key(
        std::env::var(API_KEY_ENV).ok().as_deref(),
        std::env::var(API_KEY_FILE_ENV).ok().as_deref(),
    );
    auth_headers(key.as_deref())
}

/// Shared client for the STREAMING chat path: bounded connect, unbounded body
/// (the stall timeout at the read site is what bounds it).
pub fn streaming_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(Duration::from_secs(90))
        .default_headers(env_auth_headers())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Client for NON-streamed completions: a hard total timeout is correct here.
pub fn blocking_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .default_headers(env_auth_headers())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

// ---------------------------------------------------------------------------
// Model discovery: GET <base>/v1/models on an OpenAI-compatible server.
// ---------------------------------------------------------------------------

/// One entry of `/v1/models`. `max_model_len` and `model_type` are oMLX extensions; a plain
/// OpenAI-compatible server (LM Studio, llama.cpp) omits them and they read as 0 / empty.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ListedModel {
    pub id: String,
    pub max_model_len: i64,
    pub model_type: String,
    /// Resident in memory now, per `/v1/models/status`. False also when the server has no such endpoint.
    pub loaded: bool,
}

/// `http://host:port/v1/models` for a chat endpoint such as `http://host:port/v1/chat/completions`.
/// An endpoint without a `/v1/` segment is used as a base as-is.
pub fn models_url(chat_endpoint: &str) -> String {
    let base = match chat_endpoint.find("/v1/") {
        Some(i) => &chat_endpoint[..i],
        None => chat_endpoint.trim_end_matches('/'),
    };
    format!("{base}/v1/models")
}

/// Parse the body of `/v1/models`. Entries without a string `id` are skipped; a body that is not
/// the expected `{"data":[...]}` shape is an error rather than an empty list, so the panel can say
/// "this does not look like an OpenAI server" instead of "no models".
pub fn parse_models(body: &str) -> Result<Vec<ListedModel>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("not JSON: {e}"))?;
    let arr = v["data"].as_array().ok_or("no \"data\" array in the response")?;
    Ok(arr
        .iter()
        .filter_map(|m| {
            Some(ListedModel {
                id: m["id"].as_str()?.to_string(),
                max_model_len: m["max_model_len"].as_i64().unwrap_or(0),
                model_type: m["model_type"].as_str().unwrap_or("").to_string(),
                loaded: false,
            })
        })
        .collect())
}

/// Ids of the models a `/v1/models/status` body reports as `"loaded": true`. Anything that does not
/// parse yields an empty set: the status endpoint is a bonus, never a reason to fail the list.
pub fn parse_loaded_ids(body: &str) -> std::collections::HashSet<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return Default::default() };
    v["models"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|m| m["loaded"].as_bool() == Some(true))
                .filter_map(|m| m["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `<base>/v1/models/<id>/<load|unload>` with the id percent-encoded as one path segment, so an id
/// containing `/`, `?` or `#` cannot reach another route.
pub fn model_action_url(chat_endpoint: &str, model_id: &str, load: bool) -> Result<String, String> {
    let id = model_id.trim();
    if id.is_empty() {
        return Err("no model name given".into());
    }
    // The URL parser resolves dot segments, so "." or ".." would rewrite the route instead of
    // naming a model. No real model id is either.
    if id == "." || id == ".." {
        return Err(format!("'{id}' is not a valid model name"));
    }
    let mut url = reqwest::Url::parse(&models_url(chat_endpoint)).map_err(|e| format!("bad endpoint: {e}"))?;
    url.path_segments_mut()
        .map_err(|_| "bad endpoint: cannot carry a path".to_string())?
        .push(id)
        .push(if load { "load" } else { "unload" });
    Ok(url.into())
}

/// One line for a failed load/unload. A 404 is ambiguous: FastAPI answers `{"detail":"Not Found"}`
/// when the route does not exist (a server that is not oMLX), but oMLX itself answers 404 with
/// `Model not found: <id>` for a bad name. Only the first means "this server cannot do that".
pub fn describe_action_failure(status: u16, body: &str, verb: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["detail"].as_str().map(str::to_string));
    let route_missing = matches!(status, 404 | 405) && matches!(detail.as_deref(), None | Some("Not Found"));
    if route_missing {
        return format!("this server does not offer {verb} (oMLX does); it loads models on demand or from its own UI");
    }
    let text = detail.unwrap_or_else(|| body.chars().take(200).collect());
    format!("{verb} failed ({status}): {text}")
}

/// Load or unload one model. Only oMLX offers this; a 404 says so rather than looking like a failure.
pub async fn set_model_loaded(chat_endpoint: &str, model_id: &str, load: bool) -> Result<String, String> {
    let url = model_action_url(chat_endpoint, model_id, load)?;
    let verb = if load { "load" } else { "unload" };
    let resp = blocking_client()
        .post(&url)
        .send()
        .await
        .map_err(|e| format!("cannot reach the server: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(format!("{status}: the server wants an API key. Set {API_KEY_ENV} (or {API_KEY_FILE_ENV}) in the daemon's environment and restart it"));
    }
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(describe_action_failure(status.as_u16(), &body, verb));
    }
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{verb} done"));
    Ok(message)
}

/// Ask the endpoint which models it serves. Every failure is a one-line `Err` for the panel.
pub async fn list_models(chat_endpoint: &str) -> Result<Vec<ListedModel>, String> {
    let url = models_url(chat_endpoint);
    let resp = blocking_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("cannot reach {url}: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(format!(
            "{status}: the server wants an API key. Set {API_KEY_ENV} (or {API_KEY_FILE_ENV}) in the daemon's environment and restart it"
        ));
    }
    if !status.is_success() {
        return Err(format!("{url} answered {status}"));
    }
    let body = resp.text().await.map_err(|e| format!("could not read the response: {e}"))?;
    let mut models = parse_models(&body)?;
    // Best effort: which of them are resident. A server without the endpoint just shows none.
    let status_url = format!("{url}/status");
    if let Ok(r) = blocking_client().get(&status_url).send().await {
        if r.status().is_success() {
            if let Ok(text) = r.text().await {
                let loaded = parse_loaded_ids(&text);
                for m in &mut models {
                    m.loaded = loaded.contains(&m.id);
                }
            }
        }
    }
    Ok(models)
}

// ---------------------------------------------------------------------------
// Chat messages (OpenAI shape). `content` stays a String for backward-compat
// with the existing tag path; tool fields are optional and omitted when unset.
// ---------------------------------------------------------------------------
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Present on assistant messages that request tool calls.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Present on `role:"tool"` result messages — links back to the call.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_call_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Tool definitions sent in the request (`tools` array).
// ---------------------------------------------------------------------------
#[derive(Serialize, Clone, Debug)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String, // always "function"
    pub function: FunctionDef,
}

#[derive(Serialize, Clone, Debug)]
pub struct FunctionDef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: serde_json::Value, // JSON Schema for the arguments
}

impl ToolDef {
    pub fn function(name: &str, description: &str, parameters: serde_json::Value) -> Self {
        ToolDef {
            kind: "function".into(),
            function: FunctionDef {
                name: name.into(),
                description: Some(description.into()),
                parameters,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// A complete tool call (on an assistant message and in non-streamed responses).
// OpenAI sends `arguments` as a JSON-encoded string.
// ---------------------------------------------------------------------------
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "default_function_type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

fn default_function_type() -> String {
    "function".into()
}

#[derive(Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
}

// ---------------------------------------------------------------------------
// Non-streamed response (kept for a potential non-stream fallback).
// ---------------------------------------------------------------------------
#[derive(Deserialize)]
pub struct ChatMessageResponse {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Deserialize)]
pub struct ChatChoice {
    pub message: ChatMessageResponse,
}

#[derive(Deserialize)]
pub struct ChatCompletionResponse {
    pub choices: Vec<ChatChoice>,
}

// ---------------------------------------------------------------------------
// Streaming deltas. Tool calls arrive in fragments keyed by `index`:
// the first fragment usually carries id + function.name, later fragments
// append `function.arguments`. `assemble_tool_calls` reassembles them.
// ---------------------------------------------------------------------------
#[derive(Deserialize, Debug)]
pub struct ChatDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ToolCallDelta {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<FunctionCallDelta>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct FunctionCallDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct ChatDeltaChoice {
    pub delta: ChatDelta,
}

#[derive(Deserialize, Debug)]
pub struct ChatDeltaResponse {
    pub choices: Vec<ChatDeltaChoice>,
}

/// Reassemble streamed tool-call fragments (keyed by `index`) into complete
/// `ToolCall`s. Argument fragments are concatenated in arrival order; `id` and
/// `name` are taken from whichever fragment provides them.
pub fn assemble_tool_calls(fragments: &[ToolCallDelta]) -> Vec<ToolCall> {
    use std::collections::BTreeMap;
    // index -> (id, name, arguments)
    let mut acc: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
    for f in fragments {
        let entry = acc.entry(f.index).or_default();
        if let Some(id) = &f.id {
            if !id.is_empty() {
                entry.0 = id.clone();
            }
        }
        if let Some(func) = &f.function {
            if let Some(name) = &func.name {
                if !name.is_empty() {
                    entry.1.push_str(name);
                }
            }
            if let Some(args) = &func.arguments {
                entry.2.push_str(args);
            }
        }
    }
    acc.into_iter()
        .map(|(_, (id, name, arguments))| ToolCall {
            id,
            kind: "function".into(),
            function: FunctionCall { name, arguments },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_url_is_derived_from_the_chat_endpoint() {
        assert_eq!(models_url("http://127.0.0.1:1234/v1/chat/completions"), "http://127.0.0.1:1234/v1/models");
        assert_eq!(models_url("http://h:8000/v1/completions"), "http://h:8000/v1/models");
        assert_eq!(models_url("http://h:8000/"), "http://h:8000/v1/models");
    }

    #[test]
    fn parse_models_reads_the_omlx_extensions_and_tolerates_their_absence() {
        let body = r#"{"object":"list","data":[
            {"id":"Qwen3.5-2B-4bit","model_type":"vlm","max_model_len":262144},
            {"id":"plain-openai-model"},
            {"object":"model"}
        ]}"#;
        let m = parse_models(body).unwrap();
        assert_eq!(m.len(), 2, "an entry without an id is skipped");
        assert_eq!(m[0], ListedModel { id: "Qwen3.5-2B-4bit".into(), max_model_len: 262144, model_type: "vlm".into(), loaded: false });
        assert_eq!(m[1], ListedModel { id: "plain-openai-model".into(), max_model_len: 0, model_type: String::new(), loaded: false });
    }

    #[test]
    fn parse_models_rejects_what_is_not_a_model_list() {
        assert!(parse_models("<html>").is_err());
        assert!(parse_models(r#"{"error":"API key required"}"#).is_err());
        assert_eq!(parse_models(r#"{"data":[]}"#).unwrap(), vec![]);
    }

    #[test]
    fn parse_loaded_ids_keeps_only_resident_models_and_survives_garbage() {
        let body = r#"{"model_count":3,"models":[
            {"id":"a","loaded":true},{"id":"b","loaded":false},{"id":"c"}]}"#;
        let got = parse_loaded_ids(body);
        assert_eq!(got.len(), 1);
        assert!(got.contains("a"));
        assert!(parse_loaded_ids("<html>").is_empty());
        assert!(parse_loaded_ids(r#"{"error":"x"}"#).is_empty());
    }

    #[test]
    fn model_action_url_encodes_the_id_as_a_single_segment() {
        let ep = "http://127.0.0.1:1234/v1/chat/completions";
        assert_eq!(model_action_url(ep, "Qwen3.5-2B-4bit", true).unwrap(), "http://127.0.0.1:1234/v1/models/Qwen3.5-2B-4bit/load");
        assert_eq!(model_action_url(ep, "m", false).unwrap(), "http://127.0.0.1:1234/v1/models/m/unload");
        // A hostile id must not climb out of its segment or start a query.
        let u = model_action_url(ep, "../x?y#z/w", true).unwrap();
        assert!(u.starts_with("http://127.0.0.1:1234/v1/models/"), "{u}");
        assert!(u.ends_with("/load"), "{u}");
        assert!(!u.contains('?') && !u.contains('#'), "{u}");
        assert_eq!(u.matches('/').count(), "http://127.0.0.1:1234/v1/models/X/load".matches('/').count(), "{u}");
        // Dot segments are the classic way out of a path; they are refused outright.
        assert!(model_action_url(ep, "..", true).is_err());
        assert!(model_action_url(ep, ".", false).is_err());
        assert!(model_action_url(ep, "  ", true).is_err());
    }

    #[test]
    fn a_missing_route_and_an_unknown_model_are_told_apart() {
        let missing = describe_action_failure(404, r#"{"detail":"Not Found"}"#, "load");
        assert!(missing.contains("does not offer load"), "{missing}");
        assert!(describe_action_failure(404, "<html>nope</html>", "load").contains("does not offer"));
        let unknown = describe_action_failure(404, r#"{"detail":"Model not found: x"}"#, "load");
        assert_eq!(unknown, "load failed (404): Model not found: x");
        assert_eq!(
            describe_action_failure(507, r#"{"detail":"not enough memory"}"#, "unload"),
            "unload failed (507): not enough memory"
        );
    }

    #[test]
    fn the_key_comes_from_the_environment_first_then_from_the_file() {
        let dir = std::env::temp_dir().join(format!("freecode-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("key");
        std::fs::write(&file, "from-file\nignored second line\n").unwrap();
        let f = file.to_str().unwrap();
        assert_eq!(resolve_api_key(Some("from-env"), Some(f)).as_deref(), Some("from-env"));
        assert_eq!(resolve_api_key(None, Some(f)).as_deref(), Some("from-file"));
        assert_eq!(resolve_api_key(Some("  "), Some(f)).as_deref(), Some("from-file"));
        assert_eq!(resolve_api_key(None, None), None);
        assert_eq!(resolve_api_key(None, Some("/nonexistent/freecode-key")), None);
        std::fs::write(&file, "\n").unwrap();
        assert_eq!(resolve_api_key(None, Some(f)), None, "an empty file is no key");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auth_headers_carry_a_bearer_token() {
        let h = auth_headers(Some("sk-test"));
        let v = h.get(reqwest::header::AUTHORIZATION).expect("Authorization header");
        assert_eq!(v.to_str().unwrap(), "Bearer sk-test");
        assert!(v.is_sensitive(), "the token must not appear in debug output");
    }

    #[test]
    fn auth_headers_are_absent_without_a_usable_key() {
        assert!(auth_headers(None).is_empty());
        assert!(auth_headers(Some("")).is_empty());
        assert!(auth_headers(Some("   ")).is_empty());
        // A newline would make an invalid header value; it must be dropped, not panic.
        assert!(auth_headers(Some("bad\nkey")).is_empty());
    }

    #[test]
    fn auth_headers_trim_the_key() {
        let h = auth_headers(Some("  k  \n"));
        assert_eq!(h.get(reqwest::header::AUTHORIZATION).unwrap().to_str().unwrap(), "Bearer k");
    }

    #[test]
    fn tool_message_round_trips() {
        // Assistant message requesting a tool call.
        let assistant = ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".into(),
                kind: "function".into(),
                function: FunctionCall {
                    name: "write_file".into(),
                    arguments: r#"{"path":"a.rs","content":"x"}"#.into(),
                },
            }]),
            tool_call_id: None,
        };
        let j = serde_json::to_value(&assistant).unwrap();
        assert_eq!(j["role"], "assistant");
        assert_eq!(j["tool_calls"][0]["id"], "call_1");
        assert_eq!(j["tool_calls"][0]["type"], "function");
        assert_eq!(j["tool_calls"][0]["function"]["name"], "write_file");
        assert!(j.get("tool_call_id").is_none()); // skipped when None

        // Tool result message.
        let result = ChatMessage {
            role: "tool".into(),
            content: "ok".into(),
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
        };
        let j2 = serde_json::to_value(&result).unwrap();
        assert_eq!(j2["role"], "tool");
        assert_eq!(j2["tool_call_id"], "call_1");
        assert!(j2.get("tool_calls").is_none());

        // A plain message serializes exactly as before (no tool keys leak).
        let plain = ChatMessage { role: "user".into(), content: "hi".into(), ..Default::default() };
        let j3 = serde_json::to_value(&plain).unwrap();
        assert!(j3.get("tool_calls").is_none() && j3.get("tool_call_id").is_none());
    }

    #[test]
    fn request_omits_tools_when_none() {
        let req = ChatCompletionRequest {
            model: "m".into(),
            messages: vec![],
            temperature: 0.1,
            stream: Some(true),
            tools: None,
            tool_choice: None,
        };
        let j = serde_json::to_value(&req).unwrap();
        assert!(j.get("tools").is_none() && j.get("tool_choice").is_none());

        let req2 = ChatCompletionRequest {
            tools: Some(vec![ToolDef::function("write_file", "writes a file", serde_json::json!({"type":"object"}))]),
            tool_choice: Some("auto".into()),
            ..req_base()
        };
        let j2 = serde_json::to_value(&req2).unwrap();
        assert_eq!(j2["tools"][0]["function"]["name"], "write_file");
        assert_eq!(j2["tool_choice"], "auto");
    }

    fn req_base() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "m".into(),
            messages: vec![],
            temperature: 0.1,
            stream: Some(true),
            tools: None,
            tool_choice: None,
        }
    }

    #[test]
    fn assembles_streamed_tool_calls() {
        let frags = vec![
            ToolCallDelta {
                index: 0,
                id: Some("call_1".into()),
                function: Some(FunctionCallDelta { name: Some("write_file".into()), arguments: Some("{\"path\":\"a".into()) }),
            },
            ToolCallDelta {
                index: 0,
                id: None,
                function: Some(FunctionCallDelta { name: None, arguments: Some(".rs\",\"content\":\"x\"}".into()) }),
            },
        ];
        let calls = assemble_tool_calls(&frags);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "write_file");
        let args: serde_json::Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
        assert_eq!(args["path"], "a.rs");
        assert_eq!(args["content"], "x");
    }
}
