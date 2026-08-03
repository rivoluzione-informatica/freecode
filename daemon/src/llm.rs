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

/// Shared client for the STREAMING chat path: bounded connect, unbounded body
/// (the stall timeout at the read site is what bounds it).
pub fn streaming_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Client for NON-streamed completions: a hard total timeout is correct here.
pub fn blocking_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
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
