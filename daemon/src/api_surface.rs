//! Public-API surface extraction and breaking-change diff (Rust).
//!
//! Every other gate judges the FINAL state of a file: does it parse, does it compile, do the
//! tests pass, is there a secret in it. None of them has the "before", so none of them can see
//! what *disappeared*. That leaves a real hole, observed in a live run: asked to add a function
//! and keep the existing one, a model emitted
//!
//! ```text
//! - pub fn add(a: i32, b: i32) -> i32 {   →   fn add(a: i32, b: i32) -> i32 {
//! ```
//!
//! and every gate went green. `cargo check` compiles ONE unit; demoting an item that nothing
//! inside that unit consumes is perfectly valid Rust. The breakage lands on downstream crates,
//! which do not exist at check time.
//!
//! So: parse both versions with `syn` (already a dependency, already used by the syntax
//! pre-check), reduce each to the set of items reachable from outside the crate, and diff.
//! Deterministic, in-process, milliseconds — the cheap end of COMPILOT's cheap-then-expensive.
//!
//! **Policy: this reports, it does not veto.** Narrowing an API is often exactly what was asked
//! for. A gate that blocks legitimate refactors gets switched off within a week, and a gate
//! that is off is worth nothing. Findings surface in the UI and as typed feedback the model can
//! self-correct against; `.freecode/config.json` can escalate to a hard gate for a release
//! branch.

use quote::ToTokens;
use serde::Serialize;
use std::collections::BTreeMap;

/// How far an item is visible. Only `Public` escapes the crate, but a downgrade between any two
/// levels is still a contract change worth reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Vis {
    Private,
    /// `pub(super)` / `pub(in path)` — narrower than crate-wide.
    Restricted,
    /// `pub(crate)`
    Crate,
    /// `pub`
    Public,
}

impl Vis {
    pub fn as_str(&self) -> &'static str {
        match self {
            Vis::Private => "private",
            Vis::Restricted => "restricted",
            Vis::Crate => "pub(crate)",
            Vis::Public => "pub",
        }
    }

    fn of(v: &syn::Visibility) -> Vis {
        match v {
            syn::Visibility::Public(_) => Vis::Public,
            syn::Visibility::Restricted(r) => {
                if r.path.is_ident("crate") {
                    Vis::Crate
                } else if r.path.is_ident("self") {
                    Vis::Private
                } else {
                    Vis::Restricted
                }
            }
            syn::Visibility::Inherited => Vis::Private,
        }
    }
}

/// One entry of the API surface: a stable identity (`kind` + `path`) plus the two things that
/// can change under it — how visible it is, and what its signature says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiItem {
    /// `fn`, `struct`, `enum`, `trait`, `const`, `static`, `type`, `field`, `variant`, `method`.
    pub kind: &'static str,
    /// Module-qualified, e.g. `net::Client::connect`.
    pub path: String,
    pub vis: Vis,
    /// Token-normalized signature, so pure formatting churn never registers as a change.
    pub sig: String,
}

impl ApiItem {
    fn key(&self) -> (&'static str, &str) {
        (self.kind, self.path.as_str())
    }
}

/// A change to the public contract. Additions are deliberately absent: adding API is compatible,
/// and reporting it would bury the signal that matters under noise.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum Change {
    /// The item is gone entirely (deleted, or renamed — a rename reads as removal + addition).
    Removed { kind: &'static str, path: String, vis: Vis },
    /// Still there, but reachable by fewer callers than before (`pub fn` → `fn`, and friends).
    VisibilityReduced { kind: &'static str, path: String, from: Vis, to: Vis },
    /// Same item, different contract: arity, parameter types, return type, field types.
    SignatureChanged { kind: &'static str, path: String, before: String, after: String },
}

impl Change {
    pub fn path(&self) -> &str {
        match self {
            Change::Removed { path, .. }
            | Change::VisibilityReduced { path, .. }
            | Change::SignatureChanged { path, .. } => path,
        }
    }

    /// One line, phrased for both the panel and the model's typed feedback.
    pub fn message(&self) -> String {
        match self {
            Change::Removed { kind, path, vis } => {
                format!("`{} {} {}` was removed from the public surface", vis.as_str(), kind, path)
            }
            Change::VisibilityReduced { kind, path, from, to } => format!(
                "{} `{}` was demoted from `{}` to `{}`",
                kind, path, from.as_str(), to.as_str()
            ),
            Change::SignatureChanged { kind, path, before, after } => format!(
                "{} `{}` changed signature:\n    before: {}\n    after:  {}",
                kind, path, before, after
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Normalize any AST node to a token string. `to_token_stream().to_string()` collapses
/// whitespace and comments, so reformatting an unchanged signature compares equal.
fn toks<T: ToTokens>(t: &T) -> String {
    t.to_token_stream().to_string()
}

struct Walker {
    items: Vec<ApiItem>,
}

impl Walker {
    /// `reachable` is false once we descend into a module that outside code cannot name: a `pub`
    /// item inside a private `mod` is not part of the crate's surface, and treating it as such
    /// would make every internal refactor look like a breaking change.
    fn walk(&mut self, items: &[syn::Item], prefix: &str, reachable: bool) {
        for item in items {
            match item {
                syn::Item::Fn(f) => {
                    self.push(reachable, "fn", prefix, &f.sig.ident, Vis::of(&f.vis), fn_sig(&f.sig));
                }
                syn::Item::Const(c) => {
                    self.push(reachable, "const", prefix, &c.ident, Vis::of(&c.vis), toks(&c.ty));
                }
                syn::Item::Static(s) => {
                    self.push(reachable, "static", prefix, &s.ident, Vis::of(&s.vis), toks(&s.ty));
                }
                syn::Item::Type(t) => {
                    self.push(reachable, "type", prefix, &t.ident, Vis::of(&t.vis), toks(&t.ty));
                }
                syn::Item::Struct(s) => {
                    let vis = Vis::of(&s.vis);
                    let path = self.push(reachable, "struct", prefix, &s.ident, vis, toks(&s.generics));
                    // A struct's fields are only externally settable if the struct itself is.
                    let fields_reachable = reachable && vis == Vis::Public;
                    self.fields(fields_reachable, &path, &s.fields);
                }
                syn::Item::Enum(e) => {
                    let vis = Vis::of(&e.vis);
                    let path = self.push(reachable, "enum", prefix, &e.ident, vis, toks(&e.generics));
                    // Variants inherit the enum's visibility — removing one always breaks matches.
                    for v in &e.variants {
                        self.record(
                            reachable && vis == Vis::Public,
                            "variant",
                            format!("{}::{}", path, v.ident),
                            vis,
                            toks(&v.fields),
                        );
                    }
                }
                syn::Item::Trait(t) => {
                    let vis = Vis::of(&t.vis);
                    let path = self.push(reachable, "trait", prefix, &t.ident, vis, toks(&t.generics));
                    // Trait items are as public as the trait; a changed method signature breaks
                    // every implementor.
                    for ti in &t.items {
                        if let syn::TraitItem::Fn(f) = ti {
                            self.record(
                                reachable && vis == Vis::Public,
                                "method",
                                format!("{}::{}", path, f.sig.ident),
                                vis,
                                fn_sig(&f.sig),
                            );
                        }
                    }
                }
                syn::Item::Impl(i) => {
                    // Inherent impls only. Methods of a trait impl are governed by the trait's
                    // own contract, which we already captured above (or which lives in another
                    // crate entirely).
                    if i.trait_.is_some() {
                        continue;
                    }
                    let ty = toks(&i.self_ty).replace(' ', "");
                    for ii in &i.items {
                        if let syn::ImplItem::Fn(f) = ii {
                            let vis = Vis::of(&f.vis);
                            self.record(
                                reachable,
                                "method",
                                join(prefix, &format!("{}::{}", ty, f.sig.ident)),
                                vis,
                                fn_sig(&f.sig),
                            );
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        let vis = Vis::of(&m.vis);
                        let path = join(prefix, &m.ident.to_string());
                        self.walk(inner, &path, reachable && vis == Vis::Public);
                    }
                }
                _ => {}
            }
        }
    }

    fn fields(&mut self, reachable: bool, owner: &str, fields: &syn::Fields) {
        if let syn::Fields::Named(named) = fields {
            for f in &named.named {
                if let Some(ident) = &f.ident {
                    self.record(
                        reachable,
                        "field",
                        format!("{}::{}", owner, ident),
                        Vis::of(&f.vis),
                        toks(&f.ty),
                    );
                }
            }
        }
    }

    fn push(
        &mut self,
        reachable: bool,
        kind: &'static str,
        prefix: &str,
        ident: &syn::Ident,
        vis: Vis,
        sig: String,
    ) -> String {
        let path = join(prefix, &ident.to_string());
        self.record(reachable, kind, path.clone(), vis, sig);
        path
    }

    fn record(&mut self, reachable: bool, kind: &'static str, path: String, vis: Vis, sig: String) {
        if !reachable {
            return;
        }
        self.items.push(ApiItem { kind, path, vis, sig });
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", prefix, name)
    }
}

/// Signature identity for a function: everything a caller depends on, and nothing else.
/// The body, the parameter *names*, and formatting are all excluded on purpose — renaming a
/// parameter or reformatting is not a contract change.
fn fn_sig(sig: &syn::Signature) -> String {
    let params: Vec<String> = sig
        .inputs
        .iter()
        .map(|arg| match arg {
            syn::FnArg::Receiver(r) => toks(r),
            syn::FnArg::Typed(t) => toks(&t.ty),
        })
        .collect();
    let ret = match &sig.output {
        syn::ReturnType::Default => "()".to_string(),
        syn::ReturnType::Type(_, t) => toks(t),
    };
    let asyncness = if sig.asyncness.is_some() { "async " } else { "" };
    // syn 3 replaced `unsafety: Option<Token![unsafe]>` with a three-state `Safety`. The third
    // state is real: inside an `unsafe extern` block an item can be qualified `safe`, which is
    // NOT the same public contract as an unqualified `fn`. Collapsing it to "" would make the
    // gate blind to that change, so all three states are rendered.
    let unsafety = match sig.safety {
        syn::Safety::Unsafe(_) => "unsafe ",
        syn::Safety::Safe(_) => "safe ",
        syn::Safety::Default => "",
    };
    format!(
        "{}{}({}) -> {} {}",
        asyncness,
        unsafety,
        params.join(", "),
        ret,
        toks(&sig.generics)
    )
    .trim_end()
    .to_string()
}

/// Extract the externally reachable API surface of one Rust source file.
/// `None` when the file does not parse — the Syntax Gate owns that failure, not this one.
pub fn extract(content: &str) -> Option<Vec<ApiItem>> {
    let file = syn::parse_file(content).ok()?;
    let mut w = Walker { items: Vec::new() };
    w.walk(&file.items, "", true);
    Some(w.items)
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

/// Compare two surfaces and report only what could break a caller.
///
/// Anything that was already invisible from outside (`Private`) is ignored on both sides: this
/// gate exists to protect a published contract, not to police internals.
pub fn diff(before: &[ApiItem], after: &[ApiItem]) -> Vec<Change> {
    let after_by_key: BTreeMap<_, _> = after.iter().map(|i| (i.key(), i)).collect();
    let mut changes = Vec::new();

    for b in before {
        if b.vis == Vis::Private {
            continue; // never was part of the contract
        }
        match after_by_key.get(&b.key()) {
            None => changes.push(Change::Removed {
                kind: b.kind,
                path: b.path.clone(),
                vis: b.vis,
            }),
            Some(a) => {
                if a.vis < b.vis {
                    changes.push(Change::VisibilityReduced {
                        kind: b.kind,
                        path: b.path.clone(),
                        from: b.vis,
                        to: a.vis,
                    });
                }
                if a.sig != b.sig {
                    changes.push(Change::SignatureChanged {
                        kind: b.kind,
                        path: b.path.clone(),
                        before: b.sig.clone(),
                        after: a.sig.clone(),
                    });
                }
            }
        }
    }
    changes
}

/// The gate itself: `before` → `after` for one file. Empty vec = the contract held.
/// Non-Rust files and unparseable content yield no findings (other gates cover those).
pub fn check(path: &str, before: &str, after: &str) -> Vec<Change> {
    if !path.ends_with(".rs") {
        return Vec::new();
    }
    match (extract(before), extract(after)) {
        (Some(b), Some(a)) => diff(&b, &a),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(changes: &[Change]) -> Vec<&str> {
        changes.iter().map(|c| c.path()).collect()
    }

    // --- the observed failure -------------------------------------------------

    #[test]
    fn catches_the_demotion_that_shipped_green() {
        // Verbatim from the live run: the model added `sub` and silently dropped `pub` from `add`.
        let before = "pub fn add(a: i32, b: i32) -> i32 { a + b }";
        let after = "fn add(a: i32, b: i32) -> i32 { a + b }\nfn sub(a: i32, b: i32) -> i32 { a - b }";
        let c = check("src/lib.rs", before, after);
        assert_eq!(c.len(), 1, "{:?}", c);
        assert!(matches!(
            c[0],
            Change::VisibilityReduced { from: Vis::Public, to: Vis::Private, .. }
        ));
        assert!(c[0].message().contains("add"));
    }

    // --- what must be caught --------------------------------------------------

    #[test]
    fn catches_removal() {
        let c = check("a.rs", "pub fn gone() {}\npub fn kept() {}", "pub fn kept() {}");
        assert_eq!(paths(&c), vec!["gone"]);
        assert!(matches!(c[0], Change::Removed { .. }));
    }

    #[test]
    fn catches_arity_and_type_and_return_changes() {
        let cases = [
            ("pub fn f(a: i32) {}", "pub fn f(a: i32, b: i32) {}"),      // arity
            ("pub fn f(a: i32) {}", "pub fn f(a: u64) {}"),              // param type
            ("pub fn f() -> i32 { 0 }", "pub fn f() -> u8 { 0 }"),       // return type
            ("pub fn f() {}", "pub async fn f() {}"),                    // asyncness
            ("pub fn f() {}", "pub unsafe fn f() {}"),                   // safety qualifier
            ("pub fn f<T>(a: T) {}", "pub fn f<T: Clone>(a: T) {}"),     // generic bound
        ];
        for (b, a) in cases {
            let c = check("a.rs", b, a);
            assert!(
                c.iter().any(|x| matches!(x, Change::SignatureChanged { .. })),
                "missed signature change:\n  {b}\n  {a}\n  got {c:?}"
            );
        }
    }

    #[test]
    fn catches_downgrade_to_pub_crate() {
        let c = check("a.rs", "pub fn f() {}", "pub(crate) fn f() {}");
        assert!(matches!(
            c[0],
            Change::VisibilityReduced { from: Vis::Public, to: Vis::Crate, .. }
        ));
    }

    #[test]
    fn catches_removed_public_field_and_variant() {
        let c = check(
            "a.rs",
            "pub struct S { pub a: i32, pub b: i32 }",
            "pub struct S { pub a: i32 }",
        );
        assert_eq!(paths(&c), vec!["S::b"]);

        let c = check("a.rs", "pub enum E { A, B }", "pub enum E { A }");
        assert_eq!(paths(&c), vec!["E::B"]);
    }

    #[test]
    fn catches_field_type_change_and_field_demotion() {
        let c = check(
            "a.rs",
            "pub struct S { pub a: i32 }",
            "pub struct S { pub a: String }",
        );
        assert!(matches!(c[0], Change::SignatureChanged { .. }), "{c:?}");

        let c = check("a.rs", "pub struct S { pub a: i32 }", "pub struct S { a: i32 }");
        assert!(matches!(c[0], Change::VisibilityReduced { .. }), "{c:?}");
    }

    #[test]
    fn catches_trait_method_change_breaking_every_implementor() {
        let c = check(
            "a.rs",
            "pub trait T { fn run(&self) -> i32; }",
            "pub trait T { fn run(&self, x: u8) -> i32; }",
        );
        assert_eq!(paths(&c), vec!["T::run"]);
    }

    #[test]
    fn catches_inherent_method_removal() {
        let c = check(
            "a.rs",
            "pub struct S; impl S { pub fn go(&self) {} }",
            "pub struct S; impl S {}",
        );
        assert_eq!(paths(&c), vec!["S::go"]);
    }

    #[test]
    fn catches_changes_inside_a_public_module() {
        let c = check(
            "a.rs",
            "pub mod net { pub fn connect() {} }",
            "pub mod net { fn connect() {} }",
        );
        assert_eq!(paths(&c), vec!["net::connect"]);
    }

    // --- what must NOT be caught (the half that decides whether anyone keeps it on) ---

    #[test]
    fn additions_are_compatible_and_silent() {
        let c = check("a.rs", "pub fn a() {}", "pub fn a() {}\npub fn b() {}\npub struct S;");
        assert!(c.is_empty(), "{c:?}");
    }

    #[test]
    fn private_items_are_none_of_this_gates_business() {
        let cases = [
            ("fn helper() {}", ""),                                    // deleted
            ("fn helper(a: i32) {}", "fn helper(a: u8, b: u8) {}"),     // resignatured
            ("struct S { a: i32 }", "struct S { b: u8 }"),              // reshaped
            ("mod internal { pub fn x() {} }", "mod internal { }"),     // inside a private mod
        ];
        for (b, a) in cases {
            assert!(check("a.rs", b, a).is_empty(), "flagged a private item:\n  {b}\n  {a}");
        }
    }

    #[test]
    fn body_changes_and_reformatting_are_invisible() {
        let cases = [
            ("pub fn f() -> i32 { 1 }", "pub fn f() -> i32 { 2 + 40 }"),          // body
            ("pub fn f(a:i32)->i32{a}", "pub fn f(a: i32) -> i32 {\n    a\n}"),   // formatting
            ("pub fn f(a: i32) {}", "pub fn f(renamed: i32) {}"),                 // param name
            ("pub fn f() {}", "/// docs\npub fn f() {}"),                         // doc comment
            ("pub fn a() {}\npub fn b() {}", "pub fn b() {}\npub fn a() {}"),     // reordering
        ];
        for (b, a) in cases {
            assert!(check("a.rs", b, a).is_empty(), "false positive:\n  {b}\n  {a}\n  {:?}", check("a.rs", b, a));
        }
    }

    #[test]
    fn widening_visibility_is_not_a_break() {
        assert!(check("a.rs", "fn f() {}", "pub fn f() {}").is_empty());
        assert!(check("a.rs", "pub(crate) fn f() {}", "pub fn f() {}").is_empty());
    }

    #[test]
    fn trait_impls_are_not_treated_as_inherent_api() {
        // The contract lives on the trait (often in another crate); re-emitting an impl must not
        // read as a surface change.
        let b = "pub struct S; impl Default for S { fn default() -> Self { S } }";
        let a = "pub struct S; impl Default for S { fn default() -> Self { Self } }";
        assert!(check("a.rs", b, a).is_empty());
    }

    // --- degenerate input -----------------------------------------------------

    #[test]
    fn non_rust_and_unparseable_input_yield_nothing() {
        assert!(check("a.ts", "export function f() {}", "").is_empty());
        assert!(check("a.rs", "pub fn f() {}", "fn broken( {").is_empty()); // Syntax Gate's job
        assert!(check("a.rs", "", "").is_empty());
        assert!(check("a.rs", "", "pub fn brand_new() {}").is_empty()); // new file
    }

    #[test]
    fn a_file_emptied_out_reports_every_lost_item() {
        let before = "pub fn a() {}\npub struct S { pub x: i32 }\npub enum E { V }";
        let c = check("a.rs", before, "");
        // fn a, struct S, field S::x, enum E, variant E::V
        assert_eq!(c.len(), 5, "{c:?}");
        assert!(c.iter().all(|x| matches!(x, Change::Removed { .. })));
    }
}
