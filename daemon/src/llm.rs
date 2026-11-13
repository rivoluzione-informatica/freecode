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

fn env_auth_headers() -> reqwest::header::HeaderMap {
    auth_headers(std::env::var(API_KEY_ENV).ok().as_deref())
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
            })
        })
        .collect())
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
            "{status}: the server wants an API key. Set {API_KEY_ENV} in the daemon's environment and restart it"
        ));
    }
    if !status.is_success() {
        return Err(format!("{url} answered {status}"));
    }
    let body = resp.text().await.map_err(|e| format!("could not read the response: {e}"))?;
    parse_models(&body)
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
        assert_eq!(m[0], ListedModel { id: "Qwen3.5-2B-4bit".into(), max_model_len: 262144, model_type: "vlm".into() });
        assert_eq!(m[1], ListedModel { id: "plain-openai-model".into(), max_model_len: 0, model_type: String::new() });
    }

    #[test]
    fn parse_models_rejects_what_is_not_a_model_list() {
        assert!(parse_models("<html>").is_err());
        assert!(parse_models(r#"{"error":"API key required"}"#).is_err());
        assert_eq!(parse_models(r#"{"data":[]}"#).unwrap(), vec![]);
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
