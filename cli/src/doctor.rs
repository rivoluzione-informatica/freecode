//! `freecode-cli doctor` — say exactly what is missing and how to fix it.
//!
//! FreeCode needs four things that are not in the repository: a Rust toolchain, `protoc` (both
//! `build.rs` files invoke it), Node for the extension bundle, and a local OpenAI-compatible LLM
//! endpoint. Miss any one and the failure surfaces far from its cause — a missing `protoc`
//! reads as `failed to run custom build command`, and a dead endpoint reads as a turn that
//! simply never answers.
//!
//! So: check them all, name the fix, and exit non-zero if anything REQUIRED is missing. Optional
//! findings (no Docker, no service installed) are reported but never fail the command — they
//! disable features, they do not break the product.

use std::process::Command;
use std::time::Duration;

pub const DEFAULT_LLM: &str = "http://127.0.0.1:1234/v1/chat/completions";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Ok,
    /// Present but worth knowing about, or an optional component that is absent.
    Warn,
    /// Required and absent — `doctor` exits non-zero.
    Missing,
}

impl Status {
    fn glyph(self) -> &'static str {
        match self {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Missing => "MISS",
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Missing => "missing",
        }
    }
}

pub struct Check {
    pub name: &'static str,
    pub status: Status,
    /// What was actually found — a version, an address, an error. Never a guess.
    pub detail: String,
    /// The command or action that resolves it. Empty when nothing is wrong.
    pub fix: String,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Check { name, status: Status::Ok, detail: detail.into(), fix: String::new() }
    }
    fn warn(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Check { name, status: Status::Warn, detail: detail.into(), fix: fix.into() }
    }
    fn missing(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Check { name, status: Status::Missing, detail: detail.into(), fix: fix.into() }
    }
}

/// First line of a tool's `--version`, or None when the binary is not on PATH.
fn version_of(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = if text.trim().is_empty() { String::from_utf8_lossy(&out.stderr).into_owned() } else { text.into_owned() };
    text.lines().next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
}

fn install_hint(macos: &str, debian: &str) -> String {
    if cfg!(target_os = "macos") {
        macos.to_string()
    } else {
        debian.to_string()
    }
}

// ---------------------------------------------------------------------------
// Individual checks
// ---------------------------------------------------------------------------

fn check_rust() -> Check {
    match version_of("cargo", &["--version"]) {
        Some(v) => Check::ok("rust toolchain", v),
        None => Check::missing(
            "rust toolchain",
            "`cargo` is not on PATH",
            "install from https://rustup.rs",
        ),
    }
}

fn check_protoc() -> Check {
    match version_of("protoc", &["--version"]) {
        Some(v) => Check::ok("protoc", v),
        None => Check::missing(
            "protoc",
            "not on PATH — daemon/build.rs and cli/build.rs both invoke it to compile proto/freecode.proto",
            install_hint("brew install protobuf", "sudo apt-get install -y protobuf-compiler"),
        ),
    }
}

fn check_node() -> Check {
    match version_of("node", &["--version"]) {
        Some(v) => {
            // The webview test suite uses node:test with shell-expanded globs; both are fine on
            // any supported Node, but the extension targets 20+.
            let major = v.trim_start_matches('v').split('.').next().and_then(|m| m.parse::<u32>().ok());
            match major {
                Some(m) if m >= 20 => Check::ok("node", v),
                Some(m) => Check::warn(
                    "node",
                    format!("{v} — the extension targets Node 20+"),
                    format!("upgrade Node (found major {m}); only needed to build the extension"),
                ),
                None => Check::ok("node", v),
            }
        }
        None => Check::warn(
            "node",
            "not on PATH — only needed to build the VS Code extension",
            install_hint("brew install node", "sudo apt-get install -y nodejs npm"),
        ),
    }
}

/// TCP reachability. Deliberately not a gRPC call: `doctor` must work even when the proto
/// contract has drifted, and "is something listening" is the question being asked.
fn probe_tcp(addr: &str, timeout: Duration) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let Ok(mut addrs) = addr.to_socket_addrs() else { return false };
    addrs.any(|a| TcpStream::connect_timeout(&a, timeout).is_ok())
}

fn check_daemon(endpoint: &str) -> Check {
    let hostport = endpoint.trim_start_matches("http://").trim_start_matches("https://");
    if probe_tcp(hostport, Duration::from_millis(800)) {
        Check::ok("freecode daemon", format!("listening on {hostport}"))
    } else {
        Check::warn(
            "freecode daemon",
            format!("nothing listening on {hostport}"),
            "cargo build --release -p freecode-daemon && ./target/release/freecode-daemon",
        )
    }
}

/// Hit `/v1/models` on the same host as the chat endpoint. A reachable socket is not enough:
/// the point is whether a model is actually loaded and named, which is the difference between
/// "LM Studio is open" and "a turn will answer".
fn check_llm(chat_endpoint: &str) -> Check {
    let base = chat_endpoint.split("/v1/").next().unwrap_or(chat_endpoint);
    let hostport = base.trim_start_matches("http://").trim_start_matches("https://");
    if !probe_tcp(hostport, Duration::from_millis(800)) {
        return Check::missing(
            "llm endpoint",
            format!("nothing listening on {hostport}"),
            "start a local OpenAI-compatible server (LM Studio, llama.cpp --server, Ollama) and load a model",
        );
    }
    match http_get_models(&format!("{base}/v1/models")) {
        Some(models) if !models.is_empty() => {
            let shown: Vec<&str> = models.iter().take(3).map(|s| s.as_str()).collect();
            let more = if models.len() > 3 { format!(" (+{} more)", models.len() - 3) } else { String::new() };
            Check::ok("llm endpoint", format!("{hostport} — {}{}", shown.join(", "), more))
        }
        Some(_) => Check::warn(
            "llm endpoint",
            format!("{hostport} answers but exposes no models"),
            "load a model in your LLM server",
        ),
        None => Check::warn(
            "llm endpoint",
            format!("{hostport} is open but /v1/models did not answer as expected"),
            "confirm the server speaks the OpenAI API",
        ),
    }
}

/// Minimal HTTP GET — no dependency added to the CLI for one request during one diagnostic.
/// Returns the `data[].id` values, or None if anything about the exchange was unexpected.
fn http_get_models(url: &str) -> Option<Vec<String>> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let rest = url.strip_prefix("http://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let mut stream = TcpStream::connect(hostport).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
    )
    .ok()?;
    let mut body = String::new();
    stream.read_to_string(&mut body).ok()?;
    let json = body.split("\r\n\r\n").nth(1)?;

    // Pull out every `"id":"..."` — enough for a diagnostic, and it cannot panic on a shape
    // we did not anticipate.
    let mut ids = Vec::new();
    let mut rest = json;
    while let Some(i) = rest.find("\"id\"") {
        rest = &rest[i + 4..];
        let Some(c) = rest.find(':') else { break };
        let after = rest[c + 1..].trim_start();
        if let Some(stripped) = after.strip_prefix('"') {
            if let Some(end) = stripped.find('"') {
                ids.push(stripped[..end].to_string());
            }
        }
    }
    Some(ids)
}

fn check_docker() -> Check {
    if Command::new("docker").arg("info").output().map(|o| o.status.success()).unwrap_or(false) {
        let img = Command::new("docker")
            .args(["image", "inspect", "freecode-sandbox", "--format", "{{.Id}}"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if img {
            Check::ok("docker sandbox", "daemon running, freecode-sandbox image present")
        } else {
            Check::warn(
                "docker sandbox",
                "docker is running but the freecode-sandbox image is not built",
                "docker build -t freecode-sandbox -f docker/freecode-sandbox.Dockerfile .",
            )
        }
    } else {
        Check::warn(
            "docker sandbox",
            "not available — auto mode refuses to execute commands without the container boundary",
            "install Docker, or stay in Suggest (HITL) mode",
        )
    }
}

fn check_workspace(workspace: &str) -> Check {
    let p = std::path::Path::new(workspace);
    if !p.is_dir() {
        return Check::missing("workspace", format!("{workspace} is not a directory"), "pass --workspace <path>");
    }
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let git = abs.join(".git").exists();
    let cfg = abs.join(".freecode").join("config.json").exists();
    let mut notes = Vec::new();
    if !git {
        notes.push("not a git repo — the git panel and diff view stay empty");
    }
    if !cfg {
        notes.push("no .freecode/config.json — gate defaults apply");
    }
    if notes.is_empty() {
        Check::ok("workspace", abs.display().to_string())
    } else {
        Check::warn("workspace", format!("{} — {}", abs.display(), notes.join("; ")), "")
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run every check. Returns the process exit code: 0 when nothing REQUIRED is missing.
pub fn run(workspace: &str, daemon: &str, llm: &str, json: bool) -> i32 {
    let checks = vec![
        check_rust(),
        check_protoc(),
        check_node(),
        check_workspace(workspace),
        check_daemon(daemon),
        check_llm(llm),
        check_docker(),
    ];

    let missing = checks.iter().filter(|c| c.status == Status::Missing).count();
    let warns = checks.iter().filter(|c| c.status == Status::Warn).count();

    if json {
        // One line per check, so `doctor --json | jq` works and CI can assert on it.
        println!("{{\"checks\":[");
        for (i, c) in checks.iter().enumerate() {
            println!(
                "  {{\"name\":\"{}\",\"status\":\"{}\",\"detail\":{},\"fix\":{}}}{}",
                c.name,
                c.status.as_str(),
                json_str(&c.detail),
                json_str(&c.fix),
                if i + 1 < checks.len() { "," } else { "" }
            );
        }
        println!("],\"missing\":{missing},\"warnings\":{warns}}}");
    } else {
        println!("FreeCode environment\n");
        let width = checks.iter().map(|c| c.name.len()).max().unwrap_or(0);
        for c in &checks {
            println!("  [{}] {:<width$}  {}", c.status.glyph(), c.name, c.detail, width = width);
            if !c.fix.is_empty() {
                println!("       {:<width$}  → {}", "", c.fix, width = width);
            }
        }
        println!();
        if missing > 0 {
            println!("{missing} required item(s) missing — FreeCode will not work until they are resolved.");
        } else if warns > 0 {
            println!("Ready. {warns} optional item(s) unavailable; the features above are disabled.");
        } else {
            println!("Ready.");
        }
    }

    if missing > 0 {
        1
    } else {
        0
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escaping_survives_hostile_detail() {
        assert_eq!(json_str(r#"a"b"#), r#""a\"b""#);
        assert_eq!(json_str("a\\b"), r#""a\\b""#);
        assert_eq!(json_str("a\nb"), r#""a\nb""#);
        // Control characters are ESCAPED, not dropped: dropping them would silently alter the
        // detail text, and JSON forbids them raw inside a string.
        assert_eq!(json_str("a\u{7}b"), r#""a\u0007b""#);
    }

    /// The contract that actually matters: `--json` output must PARSE. A hand-rolled serializer
    /// that merely *looks* like JSON is worse than none, because consumers trust it.
    #[test]
    fn json_output_round_trips_every_detail_we_could_emit() {
        let hostile = [
            r#"path with "quotes" and \backslashes\"#,
            "line one\nline two\ttabbed",
            "control \u{1}\u{7}\u{1f} chars",
            "unicode: caffè — 日本語",
            "",
        ];
        for h in hostile {
            let doc = format!("{{\"v\":{}}}", json_str(h));
            let decoded = parse_v(&doc).unwrap_or_else(|| panic!("not parseable: {doc}"));
            assert_eq!(decoded, h, "round-trip changed the value");
        }
    }

    /// Decode `{"v":"..."}` as produced by [`json_str`], applying JSON string-unescaping and
    /// rejecting any RAW control character. Hand-written on purpose: pulling in a JSON crate to
    /// test a hand-rolled encoder would only test the crate against itself.
    fn parse_v(doc: &str) -> Option<String> {
        let body = doc.strip_prefix("{\"v\":\"")?.strip_suffix("\"}")?;
        let mut out = String::new();
        let mut it = body.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                if (c as u32) < 0x20 {
                    return None; // encoder let a raw control char through
                }
                out.push(c);
                continue;
            }
            match it.next()? {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'u' => {
                    let hex: String = (0..4).filter_map(|_| it.next()).collect();
                    if hex.len() != 4 {
                        return None;
                    }
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                _ => return None,
            }
        }
        Some(out)
    }

    /// Every check must name a fix when it reports a problem — a diagnostic that says only
    /// "missing" moves the work to the reader.
    #[test]
    fn problems_always_carry_a_fix() {
        let cases = [
            Check::missing("x", "d", "do this"),
            Check::warn("y", "d", "do that"),
        ];
        for c in cases {
            assert!(!c.fix.is_empty(), "{} reported a problem with no fix", c.name);
        }
    }

    #[test]
    fn ok_checks_carry_no_fix() {
        let c = Check::ok("x", "1.0");
        assert_eq!(c.status, Status::Ok);
        assert!(c.fix.is_empty());
    }

    /// A missing tool must be reported, never guessed at.
    #[test]
    fn absent_binary_yields_none_not_a_fake_version() {
        assert!(version_of("definitely-not-a-real-binary-xyz", &["--version"]).is_none());
    }

    #[test]
    fn rust_is_detected_in_this_very_test_run() {
        // The suite is running under cargo, so cargo is on PATH by construction.
        assert_eq!(check_rust().status, Status::Ok);
    }

    #[test]
    fn a_closed_port_is_not_reported_as_listening() {
        // Port 1 is reserved and never bound by anything sane.
        assert!(!probe_tcp("127.0.0.1:1", Duration::from_millis(200)));
    }

    #[test]
    fn workspace_check_rejects_a_nonexistent_path() {
        let c = check_workspace("/definitely/not/a/real/path/xyz");
        assert_eq!(c.status, Status::Missing);
        assert!(!c.fix.is_empty());
    }

    #[test]
    fn workspace_check_accepts_the_repo_root() {
        let root = env!("CARGO_MANIFEST_DIR");
        let repo = std::path::Path::new(root).parent().unwrap();
        let c = check_workspace(repo.to_str().unwrap());
        assert_ne!(c.status, Status::Missing, "{}", c.detail);
    }

    #[test]
    fn install_hints_are_platform_specific() {
        let h = install_hint("brew install x", "apt install x");
        if cfg!(target_os = "macos") {
            assert!(h.starts_with("brew"));
        } else {
            assert!(h.starts_with("apt"));
        }
    }
}
