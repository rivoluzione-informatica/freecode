//! Pluggable external analyzers (occam/synapseed-style): user-registered tools
//! that receive the changed files (or a diff) and emit findings as JSON. Lets you
//! bolt on clippy / eslint / semgrep / bandit without touching the daemon.
//!
//! SECURITY: analyzer definitions are read ONLY from the GLOBAL config
//! (`~/.freecode/config.json`), never from the per-project `.freecode/config.json`.
//! A checked-out (possibly untrusted) repo must not be able to inject commands the
//! daemon would execute. Analyzers are report-only (they never auto-edit).
//!
//! Global config shape:
//! ```json
//! { "analyzers": [
//!     { "name": "clippy", "command": ["cargo","clippy","--message-format","short"],
//!       "input": "none", "extensions": ["rs"], "timeout_secs": 60 }
//! ] }
//! ```
//! Each analyzer should print a JSON array of findings on stdout:
//! `[{"severity":"error|warn|info","file":"src/x.rs","line":12,"message":"...","rule":"..."}]`

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AnalyzerConfig {
    pub name: String,
    pub command: Vec<String>,
    /// "files" (append changed paths as args, default), "diff" (pipe `git diff`
    /// to stdin), or "none".
    #[serde(default)]
    pub input: String,
    /// File extensions this analyzer applies to (empty = any).
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnalyzerFinding {
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<u64>,
    pub message: String,
    #[serde(default)]
    pub rule: Option<String>,
}

fn default_severity() -> String {
    "warn".to_string()
}

/// Parse the `analyzers` array out of a config JSON string (helper; lenient).
pub fn parse_analyzers(config_json: &str) -> Vec<AnalyzerConfig> {
    let val: serde_json::Value = match serde_json::from_str(config_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    match val.get("analyzers") {
        Some(arr) => serde_json::from_value(arr.clone()).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Read analyzer definitions from the GLOBAL config only (see module docs).
pub fn read_global_analyzers() -> Vec<AnalyzerConfig> {
    let home = match std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let path = std::path::Path::new(&home).join(".freecode").join("config.json");
    match std::fs::read_to_string(&path) {
        Ok(content) => parse_analyzers(&content),
        Err(_) => Vec::new(),
    }
}

fn ext_of(path: &str) -> &str {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
}

/// Does this analyzer apply to the given changed files?
pub fn analyzer_matches(cfg: &AnalyzerConfig, changed_files: &[String]) -> bool {
    if cfg.extensions.is_empty() {
        return cfg.input == "none" || !changed_files.is_empty();
    }
    changed_files
        .iter()
        .any(|f| cfg.extensions.iter().any(|e| e.trim_start_matches('.') == ext_of(f)))
}

fn matched_files(cfg: &AnalyzerConfig, changed_files: &[String]) -> Vec<String> {
    if cfg.extensions.is_empty() {
        changed_files.to_vec()
    } else {
        changed_files
            .iter()
            .filter(|f| cfg.extensions.iter().any(|e| e.trim_start_matches('.') == ext_of(f)))
            .cloned()
            .collect()
    }
}

/// Run one analyzer and return its findings. A non-zero exit with unparseable
/// output becomes a single `error` finding so it's still surfaced.
pub async fn run_analyzer(
    cfg: &AnalyzerConfig,
    workspace: &str,
    changed_files: &[String],
) -> Result<Vec<AnalyzerFinding>, String> {
    if cfg.command.is_empty() {
        return Err("analyzer has empty command".into());
    }

    let mut cmd = tokio::process::Command::new(&cfg.command[0]);
    cmd.args(&cfg.command[1..])
        .current_dir(workspace)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mode = cfg.input.as_str();
    if mode == "files" || mode.is_empty() {
        cmd.args(matched_files(cfg, changed_files));
    }

    let mut stdin_data: Option<String> = None;
    if mode == "diff" {
        let diff = tokio::process::Command::new("git")
            .arg("diff")
            .current_dir(workspace)
            .output()
            .await
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        stdin_data = Some(diff);
        cmd.stdin(std::process::Stdio::piped());
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn '{}': {}", cfg.command[0], e))?;

    if let Some(data) = stdin_data {
        if let Some(mut sin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = sin.write_all(data.as_bytes()).await;
            let _ = sin.shutdown().await;
        }
    }

    let timeout = std::time::Duration::from_secs(cfg.timeout_secs.max(1));
    let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("analyzer io error: {}", e)),
        Err(_) => return Err(format!("analyzer '{}' timed out after {}s", cfg.name, cfg.timeout_secs)),
    };

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    match serde_json::from_str::<Vec<AnalyzerFinding>>(stdout.trim()) {
        Ok(findings) => Ok(findings),
        Err(_) => {
            if out.status.success() {
                Ok(Vec::new())
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                let detail = if stderr.trim().is_empty() { stdout } else { stderr };
                let detail: String = detail.trim().chars().take(500).collect();
                Ok(vec![AnalyzerFinding {
                    severity: "error".into(),
                    file: None,
                    line: None,
                    message: format!("analyzer exited non-zero: {}", detail),
                    rule: Some("analyzer_failed".into()),
                }])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_global_config() {
        let json = r#"{ "analyzers": [
            { "name": "clippy", "command": ["cargo","clippy"], "extensions": ["rs"] },
            { "name": "x", "command": ["echo"], "input": "none", "timeout_secs": 5 }
        ] }"#;
        let cfgs = parse_analyzers(json);
        assert_eq!(cfgs.len(), 2);
        assert_eq!(cfgs[0].name, "clippy");
        assert_eq!(cfgs[0].extensions, vec!["rs".to_string()]);
        assert_eq!(cfgs[0].timeout_secs, 30); // default
        assert_eq!(cfgs[1].timeout_secs, 5);
        // No analyzers key -> empty.
        assert!(parse_analyzers("{}").is_empty());
    }

    #[test]
    fn extension_matching() {
        let cfg = AnalyzerConfig {
            name: "rs".into(),
            command: vec!["x".into()],
            input: "files".into(),
            extensions: vec!["rs".into()],
            timeout_secs: 30,
        };
        assert!(analyzer_matches(&cfg, &["src/a.rs".into()]));
        assert!(!analyzer_matches(&cfg, &["src/a.ts".into()]));
        assert_eq!(matched_files(&cfg, &["a.rs".into(), "b.ts".into()]), vec!["a.rs".to_string()]);
    }

    #[tokio::test]
    async fn runs_analyzer_and_parses_findings() {
        let dir = std::env::temp_dir();
        let cfg = AnalyzerConfig {
            name: "demo".into(),
            command: vec![
                "echo".into(),
                r#"[{"severity":"warn","message":"demo finding","rule":"r1"}]"#.into(),
            ],
            input: "none".into(),
            extensions: vec![],
            timeout_secs: 5,
        };
        let findings = run_analyzer(&cfg, &dir.to_string_lossy(), &[]).await.unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warn");
        assert_eq!(findings[0].message, "demo finding");
    }

    #[tokio::test]
    async fn nonzero_exit_becomes_error_finding() {
        let dir = std::env::temp_dir();
        let cfg = AnalyzerConfig {
            name: "fail".into(),
            command: vec!["false".into()],
            input: "none".into(),
            extensions: vec![],
            timeout_secs: 5,
        };
        let findings = run_analyzer(&cfg, &dir.to_string_lossy(), &[]).await.unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
        assert_eq!(findings[0].rule.as_deref(), Some("analyzer_failed"));
    }
}
