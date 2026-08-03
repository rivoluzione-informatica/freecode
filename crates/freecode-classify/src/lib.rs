//! Deterministic, typed task classifier — RFC-004 §4 step 1. No model, no network, no deps.
//!
//! Shared on purpose: the daemon's escalation ladder classifies live turns with this, and the
//! offline trajectory tooling labels training conversations with the SAME function — so an SLM
//! trained to mirror the router sees labels that match what the router will actually do.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskClass {
    Greeting,
    ChatQa,
    OutputDistill,
    TrivialEdit,
    Audit,
    Codegen,
    Unknown,
}

impl TaskClass {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskClass::Greeting => "greeting",
            TaskClass::ChatQa => "chat-qa",
            TaskClass::OutputDistill => "output-distill",
            TaskClass::TrivialEdit => "trivial-edit",
            TaskClass::Audit => "audit",
            TaskClass::Codegen => "codegen",
            TaskClass::Unknown => "unknown",
        }
    }

    /// The tier the ladder would START this class at (RFC-004 §3): cheap classes begin at the
    /// (future) local-SLM tier, heavier ones at the main model.
    pub fn starting_tier(self) -> &'static str {
        match self {
            TaskClass::Greeting => "short-circuit", // no actionable intent → handled before any tier
            TaskClass::ChatQa | TaskClass::OutputDistill | TaskClass::TrivialEdit => "T1(local-SLM)",
            // Audit = deep, whole-subsystem comprehension/review → needs the big-context model.
            TaskClass::Audit | TaskClass::Codegen | TaskClass::Unknown => "T2(main)",
        }
    }
}

/// Deterministic, typed task classifier — RFC-004 §4 step 1 (no model, no network).
pub fn classify_task(prompt: &str, mode: &str) -> TaskClass {
    if is_smalltalk(prompt) {
        return TaskClass::Greeting; // no actionable intent → short-circuited (same detector as core.rs)
    }
    if mode == "chat" {
        return TaskClass::ChatQa;
    }
    let p = prompt.to_lowercase();
    let has = |kws: &[&str]| kws.iter().any(|k| p.contains(k));

    if has(&["summar", "distill", "tl;dr", "what does this output", "what does this log", "explain the error", "explain this error"]) {
        return TaskClass::OutputDistill;
    }
    // Audit/comprehension/review — heavy whole-subsystem reading, no necessary code output. Checked
    // BEFORE codegen so audit intent wins over the raw length heuristic (audits are usually long).
    if has(&[
        "audit", "explore the", "map the", "map out", "overview of", "comprehensive overview",
        "enumerate", "analyze the", "analyze ", "inspect", "hunt for", "trace the", "review the",
        "analizza", "verifica", "esplora", "mappa ", "panoramica", "passa in rassegna",
    ]) {
        return TaskClass::Audit;
    }
    if has(&[
        "implement", "refactor", "add a feature", "add feature", "write a ", "create a ",
        "build a ", "design ", "rewrite", "port ",
        // bug-fixing is codegen intent regardless of length (short "fix the X bug" used to fall to
        // Unknown). Audit is checked first, so "audit for bugs" stays Audit.
        "fix ", "fixing", "debug", "broken", "failing", "crash", "stack trace", "patch the",
        "risolvi", "correggi", "aggiusta", "sistema il",
    ]) || prompt.len() > 600
    {
        return TaskClass::Codegen;
    }
    if has(&["rename", "typo", "bump ", "add import", "add a comment", "reformat", "one-liner", "rename the"]) {
        return TaskClass::TrivialEdit;
    }
    if (p.contains('?') || has(&["what ", "why ", "how ", "where ", "explain", "does "]))
        && !has(&["fix", "change", "edit", "update", "modify", "add ", "remove"])
    {
        return TaskClass::ChatQa;
    }
    TaskClass::Unknown
}

/// Deterministic "no actionable intent" detector (RFC-004 intent-triage). Conservative on purpose:
/// fires ONLY when the WHOLE (short) message is greeting/ack tokens, so a real request is never
/// short-circuited. "ciao" / "ok grazie" / "thanks!" → true; "fix the bug" / "ciao puoi…" → false.
pub fn is_smalltalk(prompt: &str) -> bool {
    let lower = prompt.trim().to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return true; // empty / punctuation-or-emoji only
    }
    if words.len() > 4 {
        return false; // too long to be pure small talk
    }
    const SMALLTALK: &[&str] = &[
        "ciao", "hi", "hello", "hey", "salve", "yo", "hiya", "ehi", "buongiorno", "buonasera",
        "buonanotte", "ok", "okay", "oka", "kk", "perfetto", "perfect", "great", "nice", "cool",
        "grazie", "thanks", "thank", "thx", "ty", "lol", "ahah", "haha", "bravo", "ottimo", "top",
        "wow", "good", "gg", "yes", "yep", "yeah", "no", "nope", "sure", "fine", "bene", "you",
    ];
    words.iter().all(|w| SMALLTALK.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_turns() {
        assert_eq!(classify_task("anything", "chat"), TaskClass::ChatQa);
        assert_eq!(classify_task("summarize this build log", "auto"), TaskClass::OutputDistill);
        assert_eq!(classify_task("implement a retry with backoff", "auto"), TaskClass::Codegen);
        assert_eq!(classify_task("rename the helper to compute_k", "auto"), TaskClass::TrivialEdit);
        assert_eq!(classify_task("why does the daemon panic on startup?", "auto"), TaskClass::ChatQa);
        assert_eq!(classify_task("ciao", "hitl"), TaskClass::Greeting);
        assert_eq!(classify_task("ok grazie", "auto"), TaskClass::Greeting);
    }

    #[test]
    fn audit_tasks_are_their_own_class_not_unknown_or_length_codegen() {
        // Real prompts that used to fall through to Unknown (or to Codegen via length>600).
        assert_eq!(classify_task("Audit TLS termination in src/tls.rs and src/listener.rs", "auto"), TaskClass::Audit);
        assert_eq!(classify_task("Explore the certmate codebase and map the auth flow", "auto"), TaskClass::Audit);
        assert_eq!(classify_task("Give me a comprehensive overview of the mixi project", "auto"), TaskClass::Audit);
        assert_eq!(classify_task("analizza lo stato del progetto file per file", "auto"), TaskClass::Audit);
        // audit intent beats the raw length heuristic even on a long prompt
        let long_audit = format!("Audit the WAF for false positives. {}", "x".repeat(700));
        assert_eq!(classify_task(&long_audit, "auto"), TaskClass::Audit);
        // ...but a real build request is still codegen, not audit
        assert_eq!(classify_task("implement a retry with backoff", "auto"), TaskClass::Codegen);
    }

    #[test]
    fn short_bugfix_prompts_are_codegen_not_unknown() {
        assert_eq!(classify_task("fix the auth bug", "auto"), TaskClass::Codegen);
        assert_eq!(classify_task("debug why the daemon crashes on startup", "auto"), TaskClass::Codegen);
        assert_eq!(classify_task("risolvi il test che fallisce", "auto"), TaskClass::Codegen);
        // bug-hunting framed as an audit still routes to Audit (checked first)
        assert_eq!(classify_task("audit the WAF for bugs and false positives", "auto"), TaskClass::Audit);
    }

    #[test]
    fn tiers_match_classes() {
        assert_eq!(TaskClass::OutputDistill.starting_tier(), "T1(local-SLM)");
        assert_eq!(TaskClass::Codegen.starting_tier(), "T2(main)");
        assert_eq!(TaskClass::Audit.starting_tier(), "T2(main)");
    }

    #[test]
    fn smalltalk_short_circuits_only_greetings() {
        assert!(is_smalltalk("ciao"));
        assert!(is_smalltalk("ok grazie"));
        assert!(is_smalltalk("thanks!"));
        assert!(is_smalltalk("   "));
        // real requests must NOT be short-circuited:
        assert!(!is_smalltalk("fix the auth bug"));
        assert!(!is_smalltalk("implement retry with backoff"));
        assert!(!is_smalltalk("ciao, puoi rifattorizzare il modulo auth?"));
    }
}
