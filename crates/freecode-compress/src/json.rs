//! JSON array compaction — ported *in spirit* (dependency-free) from headroom-core
//! `transforms/smart_crusher` (Apache-2.0). A large array of similar records is sampled (a
//! head + tail of elements, with the middle elided and counted) instead of dumping every
//! element — the dominant smart_crusher win for structured tool output (e.g. `cargo metadata`,
//! big JSON API/log dumps).
//!
//! Dep-free by design (freecode-compress takes no deps): a depth-aware, string-aware element
//! splitter rather than a serde parse; head/tail sampling rather than md5 content hashing.
//!
//! DEFERRED vs headroom's full smart_crusher: per-field statistics, statistical ID/score-field
//! detection, structural-outlier / error-item preservation, mixed and deeply-nested array
//! recursion, content-hash dedup. Those need a real parse and are a later increment.

use crate::estimate_tokens;

/// Elements kept verbatim from the tail of a sampled array.
const TAIL: usize = 2;
/// Below this element count we never sample (too small to be worth the elision marker).
const MIN_ELEMS: usize = 6;

/// Compress JSON whose token count exceeds `budget_tokens`: a top-level array is head+tail sampled;
/// a top-level OBJECT has each of its large array fields sampled (so a multi-array object like
/// `cargo metadata` shrinks across ALL its arrays, not just the first). Already-small / not-JSON →
/// unchanged. Anything still over budget is finished by the caller's line-importance `fit`.
pub fn compress_json(text: &str, budget_tokens: usize) -> String {
    if estimate_tokens(text) <= budget_tokens {
        return text.to_string();
    }
    let trimmed = text.trim();
    if trimmed.starts_with('[') {
        compress_array(trimmed, budget_tokens)
    } else if trimmed.starts_with('{') {
        compress_object(trimmed, budget_tokens)
    } else {
        text.to_string()
    }
}

/// Head+tail sample the first top-level `[ … ]` in `text` with a `… N of M items elided …` marker.
fn compress_array(text: &str, budget_tokens: usize) -> String {
    let (lo, hi) = match outer_array_span(text) {
        Some(span) => span,
        None => return text.to_string(),
    };
    let prefix = &text[..=lo];
    let inner = &text[lo + 1..hi];
    let suffix = &text[hi..];
    let elems = split_top_level(inner);
    let n = elems.len();
    if n <= MIN_ELEMS {
        return text.to_string();
    }

    let budget = budget_tokens as i64;
    let marker_cost = 10i64;
    let tail_start = n - TAIL;
    let mut used: i64 = estimate_tokens(prefix) as i64 + estimate_tokens(suffix) as i64 + marker_cost;
    used += elems[tail_start..].iter().map(|e| estimate_tokens(e) as i64 + 1).sum::<i64>();

    let mut head = 0usize;
    while head < tail_start {
        let c = estimate_tokens(elems[head]) as i64 + 1;
        if used + c > budget {
            break;
        }
        used += c;
        head += 1;
    }
    let elided = tail_start - head;
    if elided == 0 {
        return text.to_string();
    }

    let mut out = String::new();
    out.push_str(prefix);
    out.push('\n');
    for e in &elems[..head] {
        out.push_str(e);
        out.push_str(",\n");
    }
    out.push_str(&format!("  … {} of {} items elided …\n", elided, n));
    for (k, e) in elems[tail_start..].iter().enumerate() {
        out.push_str(e);
        if k + 1 < TAIL {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str(suffix);
    out
}

/// Sample every large array field of an object, recursing into large nested objects (depth-capped)
/// — `cargo metadata` buries its biggest arrays inside `resolve`, so a top-level-only pass misses
/// them. Keeps each `"key":` and the small scalar fields.
fn compress_object(text: &str, budget_tokens: usize) -> String {
    compress_object_depth(text, budget_tokens, 0)
}

const MAX_DEPTH: usize = 6;

fn compress_object_depth(text: &str, budget_tokens: usize, depth: usize) -> String {
    let (lo, hi) = match outer_object_span(text) {
        Some(span) => span,
        None => return text.to_string(),
    };
    let prefix = &text[..=lo];
    let body = &text[lo + 1..hi];
    let suffix = &text[hi..];
    let members = split_top_level(body);

    // A field worth compressing: a big array (sample), or a big nested object within the depth cap (recurse).
    let is_big = |v: &str| {
        (v.starts_with('[') || (depth < MAX_DEPTH && v.starts_with('{'))) && estimate_tokens(v) > 200
    };
    let big = members.iter().filter(|m| member_value(m).is_some_and(|(_, v)| is_big(v))).count();
    if big == 0 {
        return text.to_string(); // nothing large to sample here — leave to `fit`
    }
    let per = (budget_tokens / big).max(300);

    let mut out = String::new();
    out.push_str(prefix);
    out.push('\n');
    for (i, m) in members.iter().enumerate() {
        let rendered = match member_value(m) {
            Some((key, v)) if v.starts_with('[') && estimate_tokens(v) > 200 => {
                format!("{} {}", key, compress_array(v, per))
            }
            Some((key, v)) if depth < MAX_DEPTH && v.starts_with('{') && estimate_tokens(v) > 200 => {
                format!("{} {}", key, compress_object_depth(v, per, depth + 1))
            }
            _ => m.to_string(),
        };
        out.push_str("  ");
        out.push_str(rendered.trim_start());
        if i + 1 < members.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str(suffix);
    out
}

/// Split a `"key": value` member at the top-level `:` into (`"key":`, trimmed value).
fn member_value(m: &str) -> Option<(&str, &str)> {
    let b = m.as_bytes();
    let (mut in_str, mut esc, mut depth) = (false, false, 0i32);
    for (i, &c) in b.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'[' | b'{' => depth += 1,
            b']' | b'}' => depth -= 1,
            b':' if depth == 0 => return Some((&m[..=i], m[i + 1..].trim())),
            _ => {}
        }
    }
    None
}

/// Byte span `(lo, hi)` of the first top-level `[ … ]` (indices of `[` and its matching `]`),
/// skipping brackets inside JSON strings. `None` if there is no array.
fn outer_array_span(s: &str) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let mut in_str = false;
    let mut esc = false;
    let mut lo: Option<usize> = None;
    for (i, &c) in b.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'[' => {
                lo = Some(i);
                break;
            }
            _ => {}
        }
    }
    let lo = lo?;
    let mut depth = 0i32;
    in_str = false;
    esc = false;
    for (off, &c) in b[lo..].iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((lo, lo + off));
                }
            }
            _ => {}
        }
    }
    None
}

/// Byte span `(lo, hi)` of the first top-level `{ … }` (indices of `{` and its matching `}`),
/// skipping braces inside JSON strings. `None` if there is no object.
fn outer_object_span(s: &str) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let mut in_str = false;
    let mut esc = false;
    let mut lo: Option<usize> = None;
    for (i, &c) in b.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => {
                lo = Some(i);
                break;
            }
            _ => {}
        }
    }
    let lo = lo?;
    let mut depth = 0i32;
    in_str = false;
    esc = false;
    for (off, &c) in b[lo..].iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((lo, lo + off));
                }
            }
            _ => {}
        }
    }
    None
}

/// Split `inner` (the contents between the array brackets) on top-level commas — depth 0 over
/// both `[]` and `{}`, skipping commas inside strings. Each element is trimmed.
fn split_top_level(inner: &str) -> Vec<&str> {
    let b = inner.as_bytes();
    let mut elems = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let mut start = 0usize;
    for (i, &c) in b.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'[' | b'{' => depth += 1,
            b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                elems.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = inner[start..].trim();
    if !last.is_empty() {
        elems.push(last);
    }
    elems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_array_unchanged() {
        let t = r#"[{"a":1},{"a":2},{"a":3}]"#;
        assert_eq!(compress_json(t, 5), t); // n <= MIN_ELEMS
    }

    #[test]
    fn samples_large_array_and_keeps_tail() {
        let mut s = String::from("[");
        for i in 0..500 {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(r#"{{"id":{},"name":"item {}","ok":true}}"#, i, i));
        }
        s.push(']');
        let out = compress_json(&s, 300);
        assert!(estimate_tokens(&out) <= 300 + 40, "must fit the budget");
        assert!(out.contains("items elided"), "middle should be elided with a count");
        assert!(out.contains(r#""id":499"#), "the tail element must survive");
        assert!(out.contains("of 500 items"), "should report the true total");
    }

    #[test]
    fn split_respects_nested_braces_and_strings() {
        // Commas inside nested objects and inside strings must NOT split elements.
        let inner = r#"{"a":[1,2,3]},{"b":"x,y,z"},{"c":1}"#;
        let elems = split_top_level(inner);
        assert_eq!(elems.len(), 3, "got {:?}", elems);
        assert_eq!(elems[1], r#"{"b":"x,y,z"}"#);
    }

    #[test]
    fn non_array_unchanged() {
        let t = "this is just a long line of prose with no json array in it at all, repeated.";
        assert_eq!(compress_json(t, 1), t);
    }

    #[test]
    fn compresses_every_array_field_of_an_object() {
        // An object with TWO big arrays (like `cargo metadata`) — both must shrink, not just the first.
        let arr = |tag: &str| {
            let mut s = String::from("[");
            for i in 0..400 {
                if i > 0 {
                    s.push(',');
                }
                s.push_str(&format!(r#"{{"id":{},"v":"{}_{}"}}"#, i, tag, i));
            }
            s.push(']');
            s
        };
        let obj = format!(r#"{{"packages": {}, "members": {}, "version": 1}}"#, arr("A"), arr("B"));
        let out = compress_json(&obj, 400);
        assert!(out.matches("items elided").count() >= 2, "each array field should be sampled: {out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&obj) / 2, "must compress hard, got {}", estimate_tokens(&out));
        assert!(out.contains(r#""version": 1"#), "small scalar fields kept: {out}");
    }
}
