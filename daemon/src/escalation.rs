//! RFC-004 Slice 0 — escalation-ladder TELEMETRY ONLY (no behavior change).
//!
//! The deterministic task classifier now lives in the shared `freecode-classify` crate (so the live
//! router and the offline training labels agree); this module re-exports it and adds the RFC-004
//! telemetry: logging (a) the tier the ladder *would* start a turn at and (b) where a gate-driven
//! escalation *would* trigger. Everything here is observation — it never changes which model runs.
//! Gated behind `GateConfig.escalation_telemetry`.

pub use freecode_classify::{classify_task, is_smalltalk, TaskClass};

/// Turn-start telemetry line.
pub fn log_turn_class(class: TaskClass, mode: &str) {
    println!(
        "[rfc004-slice0] turn class={} mode={} would_start_tier={}",
        class.as_str(),
        mode,
        class.starting_tier()
    );
}

/// Gate-driven escalation telemetry line: a gate failed, so the ladder WOULD escalate the tier.
pub fn log_escalation_signal(class: TaskClass, gate: &str, attempt: usize) {
    println!(
        "[rfc004-slice0] escalation WOULD trigger: gate=\"{}\" failed → escalate from {} (class={}, attempt={})",
        gate,
        class.starting_tier(),
        class.as_str(),
        attempt
    );
}

/// RFC-004 PIC-1 — TELEMETRY ONLY: log what the deterministic 3-way router (`freecode_verdict::route`)
/// WOULD do with a finished turn, derived from its hard-gate outcome. It drives nothing yet (no
/// behavior change) — it validates the verdict spine against the real loop and lets us measure how
/// often the "uncertain → escalate to T2" band would fire before wiring it live.
pub fn log_route(outcome: &str, retry_count: usize, max_retries: usize, safety_blocks: usize, session_id: &str) {
    use freecode_verdict::{route, HardVerdict};
    let route_str = match outcome {
        // retries exhausted on a compile / regression / safety failure = a hard gate ultimately blocked.
        "unresolved" => route(&[HardVerdict::Veto("hard gates unsatisfied after retries".to_string())], retry_count, max_retries).as_str(),
        // a clean, gate-passing result (possibly after self-correction).
        "resolved" => route(&[HardVerdict::Pass], retry_count, max_retries).as_str(),
        // infra / abort / model error — not a gate verdict, nothing to route.
        _ => "n/a",
    };
    println!("[rfc004-route] outcome={outcome} retries={retry_count}/{max_retries} safety_blocks={safety_blocks} → would_route={route_str} session={session_id}");
    persist_route(outcome, route_str, retry_count, max_retries, safety_blocks, session_id);
}

/// PIC-2 — persist the route decision to an append-only JSONL so the "uncertain→escalate" band is
/// MEASURABLE (the telemetry println goes to stdout, which is dropped when the daemon runs detached).
/// Best-effort: never panics, never blocks a turn on a write failure. See [`route_log_path`] for
/// where it lands.
/// Where the routing JSONL lands, in precedence order:
///
/// 1. `$FREECODE_ROUTE_LOG` — always wins, on every platform.
/// 2. macOS: `~/Library/Logs/freecode-route.jsonl`, the platform's own convention.
/// 3. Elsewhere: `$XDG_STATE_HOME/freecode/route.jsonl`, falling back to
///    `~/.local/state/freecode/route.jsonl`. State, not cache and not config — it is machine-local
///    history that should survive a reboot but means nothing on another machine.
/// 4. No home at all (a container without `$HOME`): `/tmp`, so telemetry never takes a turn down.
///
/// This used to hardcode the macOS path unconditionally, which on Linux created a `~/Library`
/// directory that belongs to no convention there.
pub fn route_log_path() -> String {
    if let Ok(explicit) = std::env::var("FREECODE_ROUTE_LOG") {
        if !explicit.trim().is_empty() {
            return explicit;
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if home.is_empty() {
        return "/tmp/freecode-route.jsonl".to_string();
    }
    if cfg!(target_os = "macos") {
        return format!("{home}/Library/Logs/freecode-route.jsonl");
    }
    let state = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{home}/.local/state"));
    format!("{state}/freecode/route.jsonl")
}

fn persist_route(outcome: &str, route_str: &str, retry_count: usize, max_retries: usize, safety_blocks: usize, session_id: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let rec = serde_json::json!({
        "ts": ts, "kind": "rfc004-route", "outcome": outcome, "would_route": route_str,
        "retries": retry_count, "max_retries": max_retries, "safety_blocks": safety_blocks, "session": session_id,
    });
    let path = route_log_path();
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let _ = writeln!(f, "{rec}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_telemetry_maps_and_persists() {
        let p = std::env::temp_dir().join("freecode_route_pic2_test.jsonl");
        let _ = std::fs::remove_file(&p);
        std::env::set_var("FREECODE_ROUTE_LOG", &p);
        log_route("resolved", 0, 3, 0, "sess_t"); // clean -> ship
        log_route("unresolved", 3, 3, 1, "sess_t"); // gates unsatisfied, retries exhausted -> escalate
        log_route("connection_error", 0, 3, 0, "sess_t"); // infra -> n/a
        std::env::remove_var("FREECODE_ROUTE_LOG");
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(body.contains("\"would_route\":\"ship\""), "resolved -> ship; got: {body}");
        assert!(body.contains("\"would_route\":\"escalate-to-T2\""), "unresolved exhausted -> escalate; got: {body}");
        assert!(body.contains("\"would_route\":\"n/a\""), "infra outcome -> n/a; got: {body}");
        assert_eq!(body.lines().count(), 3, "one JSONL record per turn");
        let _ = std::fs::remove_file(&p);
    }

    /// An explicit override wins everywhere — it is the escape hatch for containers, CI, and
    /// anyone whose home layout we did not anticipate.
    #[test]
    fn explicit_env_var_always_wins() {
        std::env::set_var("FREECODE_ROUTE_LOG", "/somewhere/explicit.jsonl");
        assert_eq!(route_log_path(), "/somewhere/explicit.jsonl");
        std::env::remove_var("FREECODE_ROUTE_LOG");
    }

    /// An empty override is a mistake, not an instruction to write to "".
    #[test]
    fn empty_env_var_falls_through_to_the_default() {
        std::env::set_var("FREECODE_ROUTE_LOG", "   ");
        let p = route_log_path();
        assert!(!p.trim().is_empty(), "empty override produced an empty path");
        assert!(p.ends_with(".jsonl"), "unexpected default: {p}");
        std::env::remove_var("FREECODE_ROUTE_LOG");
    }

    /// Regression: this used to hardcode `~/Library/Logs` on every platform, creating a directory
    /// on Linux that belongs to no convention there.
    #[test]
    fn default_path_follows_the_platform() {
        std::env::remove_var("FREECODE_ROUTE_LOG");
        let p = route_log_path();
        if cfg!(target_os = "macos") {
            assert!(p.contains("/Library/Logs/"), "macOS should use Library/Logs: {p}");
        } else {
            assert!(!p.contains("/Library/"), "non-macOS must not invent ~/Library: {p}");
            assert!(p.contains("/freecode/"), "should be namespaced under freecode/: {p}");
        }
    }

    #[test]
    fn xdg_state_home_is_honoured_off_macos() {
        if cfg!(target_os = "macos") {
            return; // macOS has its own convention and does not consult XDG
        }
        std::env::remove_var("FREECODE_ROUTE_LOG");
        std::env::set_var("XDG_STATE_HOME", "/xdg/state");
        assert_eq!(route_log_path(), "/xdg/state/freecode/route.jsonl");
        std::env::remove_var("XDG_STATE_HOME");
    }
}
