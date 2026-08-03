//! freecode-verdict — the deterministic verdict spine for an edit/turn (RFC-004 router foundation).
//!
//! It is a HARD AND-chain with a **typed firewall**: a [`HardVerdict::Veto`] is dispositive and can
//! never be outvoted, summed, or discounted. There is intentionally NO way to turn a Veto into a
//! weighable score — so no amount of positive evidence can make a turn `Ship` once any hard gate
//! vetoes (a non-compiling, secret-leaking, or regressed edit can never "score positive").
//!
//! Soft, model-sourced belief aggregation (integer log-odds + correlation-aware discounting, à la
//! AIMP L3) is deliberately **deferred**: it only earns its weight once a model-family-correlated
//! soft voter (e.g. a T1/freelm self-rating) actually enters the verdict. Until then, the strongest
//! mitigation of the closed-validation-loop risk is to NOT admit such a voter at all — and this
//! crate is the firewall that must provably hold before one ever is admitted.

/// A dispositive gate result. `Veto` is absolute. There is no `LogOdds`/numeric constructor by
/// design: a hard gate can only pass or veto — it can never become a term in a weighted sum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HardVerdict {
    Pass,
    /// The gate blocks the turn unconditionally; the string is the human/agent-facing reason.
    Veto(String),
}

impl HardVerdict {
    pub fn is_veto(&self) -> bool {
        matches!(self, HardVerdict::Veto(_))
    }
}

/// What the router would do with a turn's verdict (RFC-004 escalation ladder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// All hard gates passed (and, once soft belief exists, the soft band accepted) → apply/answer.
    Ship,
    /// A gate vetoed and retries remain → let the SAME tier try again with the veto reasons.
    RetrySameTier,
    /// A gate vetoed and retries are exhausted → escalate to the stronger model (T2).
    EscalateToT2,
}

impl Route {
    pub fn as_str(self) -> &'static str {
        match self {
            Route::Ship => "ship",
            Route::RetrySameTier => "retry-same-tier",
            Route::EscalateToT2 => "escalate-to-T2",
        }
    }
}

/// Route a turn from its HARD verdicts only (soft belief deferred — see module docs).
///
/// The firewall, stated as code: **if ANY gate vetoed, the turn cannot `Ship`** — full stop,
/// regardless of how many gates passed. With a veto present, retries-left decides retry vs escalate.
pub fn route(hard: &[HardVerdict], retries_used: usize, max_retries: usize) -> Route {
    if hard.iter().any(HardVerdict::is_veto) {
        return if retries_used < max_retries {
            Route::RetrySameTier
        } else {
            Route::EscalateToT2
        };
    }
    Route::Ship
}

/// The veto reasons that fired (for telemetry / agent feedback). Empty when nothing vetoed.
pub fn veto_reasons(hard: &[HardVerdict]) -> Vec<&str> {
    hard.iter()
        .filter_map(|h| match h {
            HardVerdict::Veto(r) => Some(r.as_str()),
            HardVerdict::Pass => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// COMPILOT-validated mechanics (PACT'25, Merouani/Baghdadi): typed feedback + cheap-then-expensive
// validation + best-of-N. These are the pure, deterministic cores; the daemon wires them to its
// (async) gate stages. The principles: a typed, raw-detail reason grounds the model's self-correction
// (in-context learning); a cheap check runs before a costly one and short-circuits; sampling N
// candidates and keeping the first that passes beats a single attempt against a stochastic generator.
// ---------------------------------------------------------------------------------------------

/// Typed veto category — COMPILOT's feedback classes mapped onto freecode's gates. Variants are
/// declared in COST order (cheapest first) so `derive(Ord)` lets the validator run cheap→expensive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VetoKind {
    /// Parse / AST-invalid — the cheapest check (no compiler invocation).
    Syntax,
    /// Deterministic content scan: secrets, hidden/bidi chars, merge markers.
    Safety,
    /// Prompt-injection hit in the proposed content.
    Injection,
    /// Blast-radius / permission tier (forces HITL) — a policy veto.
    Policy,
    /// Type-check failure.
    Type,
    /// Compiler error.
    Compile,
    /// Was-green-now-red: a project that compiled before this turn now fails.
    Regression,
    /// Affected tests fail — the most expensive check.
    Test,
}

impl VetoKind {
    /// Stable reason code fed back to the model (matches the daemon's `[reason: …]` codes).
    pub fn reason_code(self) -> &'static str {
        match self {
            VetoKind::Syntax => "syntax_error",
            VetoKind::Safety => "safety",
            VetoKind::Injection => "injection",
            VetoKind::Policy => "policy",
            VetoKind::Type => "type_error",
            VetoKind::Compile => "compile_error",
            VetoKind::Regression => "regression",
            VetoKind::Test => "test_failure",
        }
    }
}

/// A typed veto carrying the RAW detail (COMPILOT: feed raw diagnostics, not summaries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Veto {
    pub kind: VetoKind,
    pub detail: String,
}

impl Veto {
    pub fn new(kind: VetoKind, detail: impl Into<String>) -> Self {
        Veto { kind, detail: detail.into() }
    }
    /// The reason-coded feedback block the model self-corrects against.
    pub fn feedback(&self) -> String {
        format!("[reason: {}] {}", self.kind.reason_code(), self.detail)
    }
}

/// CHEAP-THEN-EXPENSIVE: run gate stages in the given (cost-ordered) sequence and STOP at the first
/// veto, returning it. Each stage is a thunk (`FnOnce`), so a later, expensive stage is NEVER
/// executed once a cheaper one vetoes — the daemon must pass them cheapest-first. Returns `None`
/// (all clear) only when every stage passed.
pub fn validate_ordered<F>(stages: Vec<F>) -> Option<Veto>
where
    F: FnOnce() -> Option<Veto>,
{
    for stage in stages {
        if let Some(v) = stage() {
            return Some(v);
        }
    }
    None
}

/// Result of a best-of-N selection over N candidate verdicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BestOfN {
    /// Index of the first candidate that PASSED (no veto), or `None` if all failed → escalate to T2.
    pub picked: Option<usize>,
    /// How many candidates vetoed.
    pub vetoed: usize,
}

/// BEST-OF-N: given N candidates' verdicts (`None` = passed, `Some(veto)` = failed, in caller-defined
/// order), pick the FIRST that passed. If all failed, `picked` is `None` → the turn escalates to T2
/// (RFC-006 §4: "if none pass → escalate"). Deterministic; order encodes preference.
pub fn select_best_of_n(candidate_verdicts: &[Option<Veto>]) -> BestOfN {
    BestOfN {
        picked: candidate_verdicts.iter().position(Option::is_none),
        vetoed: candidate_verdicts.iter().filter(|v| v.is_some()).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass_n(n: usize) -> Vec<HardVerdict> {
        vec![HardVerdict::Pass; n]
    }

    #[test]
    fn all_pass_ships() {
        assert_eq!(route(&pass_n(5), 0, 3), Route::Ship);
        assert_eq!(route(&[], 0, 3), Route::Ship); // no gates at all → nothing blocks
    }

    #[test]
    fn the_firewall_one_veto_can_never_ship_under_any_pile_of_passes() {
        // This is the load-bearing safety property: a single Veto is un-overridable by ANY quantity
        // of passing gates, at ANY retry count. (There is no numeric term that could outvote it —
        // the type itself forbids it.)
        for passes in [0usize, 1, 10, 1000] {
            for retries in [0usize, 1, 3, 99] {
                let mut gates = pass_n(passes);
                gates.push(HardVerdict::Veto("rustc: E0308".into()));
                gates.extend(pass_n(passes)); // veto surrounded by passes on both sides
                assert_ne!(
                    route(&gates, retries, 3),
                    Route::Ship,
                    "a Veto must never Ship — passes={passes} retries={retries}"
                );
            }
        }
    }

    #[test]
    fn veto_retries_then_escalates() {
        let v = vec![HardVerdict::Veto("regression".into())];
        assert_eq!(route(&v, 0, 3), Route::RetrySameTier);
        assert_eq!(route(&v, 2, 3), Route::RetrySameTier);
        assert_eq!(route(&v, 3, 3), Route::EscalateToT2); // exhausted
        assert_eq!(route(&v, 9, 3), Route::EscalateToT2);
    }

    #[test]
    fn veto_reasons_collects_only_vetoes() {
        let g = vec![
            HardVerdict::Pass,
            HardVerdict::Veto("compile".into()),
            HardVerdict::Pass,
            HardVerdict::Veto("secret".into()),
        ];
        assert_eq!(veto_reasons(&g), vec!["compile", "secret"]);
        assert!(veto_reasons(&pass_n(3)).is_empty());
    }

    // --- COMPILOT mechanics ---

    #[test]
    fn veto_kind_reason_codes_and_feedback() {
        assert_eq!(VetoKind::Syntax.reason_code(), "syntax_error");
        assert_eq!(VetoKind::Compile.reason_code(), "compile_error");
        assert_eq!(VetoKind::Test.reason_code(), "test_failure");
        assert_eq!(
            Veto::new(VetoKind::Compile, "E0308 mismatched types").feedback(),
            "[reason: compile_error] E0308 mismatched types"
        );
    }

    #[test]
    fn veto_kinds_are_ordered_cheap_to_expensive() {
        // The declaration order IS the cost order — the validator relies on it.
        assert!(VetoKind::Syntax < VetoKind::Compile);
        assert!(VetoKind::Compile < VetoKind::Test);
        assert!(VetoKind::Safety < VetoKind::Type);
        let mut kinds = vec![VetoKind::Test, VetoKind::Syntax, VetoKind::Compile];
        kinds.sort();
        assert_eq!(kinds, vec![VetoKind::Syntax, VetoKind::Compile, VetoKind::Test]);
    }

    #[test]
    fn validate_ordered_short_circuits_and_never_runs_the_expensive_stage() {
        use std::cell::Cell;
        let expensive_ran = Cell::new(false);
        let v = validate_ordered(vec![
            Box::new(|| None) as Box<dyn FnOnce() -> Option<Veto>>,                       // cheap: pass
            Box::new(|| Some(Veto::new(VetoKind::Syntax, "unbalanced brace"))),           // cheap: VETO
            Box::new(|| { expensive_ran.set(true); Some(Veto::new(VetoKind::Compile, "x")) }), // expensive: must NOT run
        ]);
        assert_eq!(v, Some(Veto::new(VetoKind::Syntax, "unbalanced brace")), "first veto returned");
        assert!(!expensive_ran.get(), "the expensive stage must be short-circuited");
    }

    #[test]
    fn validate_ordered_returns_first_veto_and_passes_when_all_clear() {
        let none = validate_ordered::<Box<dyn FnOnce() -> Option<Veto>>>(vec![
            Box::new(|| None),
            Box::new(|| None),
        ]);
        assert_eq!(none, None, "all stages clear → no veto");
        let first = validate_ordered(vec![
            Box::new(|| Some(Veto::new(VetoKind::Type, "first"))) as Box<dyn FnOnce() -> Option<Veto>>,
            Box::new(|| Some(Veto::new(VetoKind::Test, "second"))),
        ]);
        assert_eq!(first.unwrap().kind, VetoKind::Type, "stops at the FIRST veto");
    }

    #[test]
    fn best_of_n_picks_first_pass_else_escalates() {
        let none = || None::<Veto>;
        let veto = |k| Some(Veto::new(k, ""));
        // first candidate passes
        assert_eq!(select_best_of_n(&[none(), veto(VetoKind::Compile)]), BestOfN { picked: Some(0), vetoed: 1 });
        // first fails, second passes
        assert_eq!(select_best_of_n(&[veto(VetoKind::Syntax), none(), none()]), BestOfN { picked: Some(1), vetoed: 1 });
        // ALL fail → escalate (picked None)
        assert_eq!(
            select_best_of_n(&[veto(VetoKind::Compile), veto(VetoKind::Test)]),
            BestOfN { picked: None, vetoed: 2 }
        );
        // empty → nothing to pick, nothing vetoed
        assert_eq!(select_best_of_n(&[]), BestOfN { picked: None, vetoed: 0 });
    }
}
