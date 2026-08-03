//! Knee-point adaptive sizing — ported *in spirit* (dependency-free) from headroom-core
//! `transforms/adaptive_sizer.rs` (Apache-2.0). Keeps the Kneedle knee detection over a
//! cumulative unique word-bigram coverage curve; drops headroom's zlib-ratio validation
//! tier and the SimHash estimator (uses an exact distinct-line count instead — deterministic
//! and free of false near-duplicates).

use std::collections::HashSet;

/// Pick how many of `items` (in importance order) to keep, by locating the "knee" of the
/// information-coverage curve. `bias` > 1 keeps more, < 1 compresses harder. Result is
/// clamped to `[min_k, max_k]` (`max_k = None` → up to `items.len()`).
pub fn compute_optimal_k(items: &[&str], bias: f64, min_k: usize, max_k: Option<usize>) -> usize {
    let n = items.len();
    let effective_max = max_k.unwrap_or(n);
    // Tier 1: small inputs are kept whole.
    if n <= 8 {
        return n;
    }
    // Near-total redundancy: at most 3 distinct lines → keep that many.
    let unique = unique_count(items);
    if unique <= 3 {
        return min_k.max(unique).min(effective_max);
    }
    // Tier 2: Kneedle on the bigram-coverage curve.
    let curve = compute_unique_bigram_curve(items);
    let knee = find_knee(&curve);
    let diversity = unique as f64 / n as f64; // fraction genuinely distinct
    let knee = match knee {
        // No saturation: scale keep-fraction with diversity (1.0→100%, 0.0→30%).
        None => min_k.max((n as f64 * (0.3 + 0.7 * diversity)) as usize),
        // Knee found but high diversity: apply a diversity floor.
        Some(k) if diversity > 0.7 => {
            let floor = min_k.max((n as f64 * (0.3 + 0.7 * diversity)) as usize);
            k.max(floor)
        }
        Some(k) => k,
    };
    let k = min_k.max((knee as f64 * bias) as usize);
    min_k.max(k.min(effective_max))
}

/// Kneedle knee of a monotonically-increasing curve: the index of maximum deviation above
/// the start→end diagonal. Returns a 1-indexed "keep this many" count, or `None` if the
/// curve never deviates more than 0.05 from the diagonal (no clear knee).
pub fn find_knee(curve: &[usize]) -> Option<usize> {
    let n = curve.len();
    if n < 3 {
        return None;
    }
    let y_min = curve[0] as f64;
    let y_max = curve[n - 1] as f64;
    if (y_max - y_min).abs() < f64::EPSILON {
        return Some(1); // flat curve — everything identical
    }
    let x_range = (n - 1) as f64;
    let y_range = y_max - y_min;
    let mut max_diff = -1.0f64;
    let mut knee_idx: Option<usize> = None;
    for (i, &y) in curve.iter().enumerate() {
        let x_norm = i as f64 / x_range;
        let y_norm = (y as f64 - y_min) / y_range;
        let diff = y_norm - x_norm;
        if diff > max_diff {
            max_diff = diff;
            knee_idx = Some(i);
        }
    }
    if max_diff < 0.05 {
        return None;
    }
    knee_idx.map(|i| i + 1)
}

/// Cumulative unique-bigram coverage: each item contributes its lowercased word-level
/// bigrams (single-word items contribute `(word, "")`); `curve[k]` is the running count of
/// distinct bigrams after seeing `items[0..=k]`.
pub fn compute_unique_bigram_curve(items: &[&str]) -> Vec<usize> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut curve: Vec<usize> = Vec::with_capacity(items.len());
    for item in items {
        let lower = item.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();
        if words.len() == 1 {
            seen.insert((words[0].to_string(), String::new()));
        } else {
            for w in words.windows(2) {
                seen.insert((w[0].to_string(), w[1].to_string()));
            }
        }
        curve.push(seen.len());
    }
    curve
}

fn unique_count(items: &[&str]) -> usize {
    items.iter().copied().collect::<HashSet<&str>>().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knee_on_saturating_curve() {
        // Rises fast then flattens → knee early in the curve.
        let curve = vec![0, 5, 9, 11, 12, 12, 12, 12, 12, 12];
        let knee = find_knee(&curve).expect("should find a knee");
        assert!(knee <= 5, "knee should land in the steep region, got {knee}");
    }

    #[test]
    fn optimal_k_small_input_kept_whole() {
        let items = ["a", "b", "c"];
        assert_eq!(compute_optimal_k(&items, 1.0, 2, None), 3);
    }

    #[test]
    fn optimal_k_high_redundancy_collapses() {
        // 50 identical lines → ≤3 unique → keep min_k.
        let dup: Vec<&str> = std::iter::repeat_n("same line here", 50).collect();
        let k = compute_optimal_k(&dup, 1.0, 5, None);
        assert!(k <= 5, "near-total redundancy should collapse hard, got {k}");
    }
}
