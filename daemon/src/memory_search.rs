use std::path::Path;
use std::collections::{HashSet, HashMap};

#[derive(serde::Deserialize, Clone, Debug)]
pub struct MemoryItem {
    pub id: String,
    pub content: String,
}

struct Document {
    content: String,
    tokens: Vec<String>,
}

pub fn rank_documents(documents: &[String], query: &str, top_k: usize) -> Vec<String> {
    if documents.is_empty() {
        return Vec::new();
    }

    // Standard list of common English stopwords
    let stopwords: HashSet<&str> = [
        "the", "a", "an", "and", "or", "but", "if", "then", "of", "to", "in", "on", "for", "with", "is", "was", "were", "be", "been", "are",
        "you", "i", "we", "they", "he", "she", "it", "this", "that", "how", "what", "where", "when", "why", "who", "do", "does", "did",
        "have", "has", "had", "can", "could", "would", "should", "will", "about", "my", "your", "our", "their", "his", "her",
        "its", "from", "by", "as", "at", "please", "ready", "just", "very", "so"
    ].iter().cloned().collect();

    let tokenize = |text: &str| -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .filter(|s| !stopwords.contains(s))
            .map(|s| s.to_string())
            .collect()
    };

    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return Vec::new();
    }

    let docs: Vec<Document> = documents.iter().map(|d| Document {
        content: d.clone(),
        tokens: tokenize(d),
    }).collect();

    let total_docs = docs.len();

    // Document frequency (DF) for each query token
    let mut df = HashMap::new();
    for token in &query_tokens {
        let count = docs.iter().filter(|d| d.tokens.contains(token)).count();
        df.insert(token.clone(), count);
    }

    let total_len: usize = docs.iter().map(|d| d.tokens.len()).sum();
    let avgdl = total_len as f64 / total_docs as f64;

    // BM25 parameters
    let k1 = 1.2;
    let b = 0.75;

    let mut scored_docs: Vec<(f64, String)> = Vec::new();

    for doc in docs {
        let mut score = 0.0;
        let doc_len = doc.tokens.len() as f64;

        for token in &query_tokens {
            let df_val = *df.get(token).unwrap_or(&0);
            if df_val == 0 {
                continue;
            }

            // Smoothed positive-guaranteed IDF: ln((N + 1) / (df + 0.5))
            let idf = ((total_docs as f64 + 1.0) / (df_val as f64 + 0.5)).ln();

            let tf = doc.tokens.iter().filter(|t| *t == token).count() as f64;

            if tf > 0.0 {
                // BM25 term frequency scaling formula
                let tf_component = tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * (doc_len / avgdl)));
                score += idf * tf_component;
            }
        }

        if score > 0.0 {
            scored_docs.push((score, doc.content));
        }
    }

    // Sort by score descending
    scored_docs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored_docs.into_iter().take(top_k).map(|(_, content)| content).collect()
}

pub fn search_project_memories(workspace_path: &str, query: &str, top_k: usize) -> Vec<String> {
    let project_mem_path = Path::new(workspace_path).join(".freecode").join("project_memory.json");
    if !project_mem_path.exists() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&project_mem_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let items: Vec<MemoryItem> = match serde_json::from_str(&content) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };
    let doc_strings: Vec<String> = items.into_iter().map(|item| item.content).collect();
    rank_documents(&doc_strings, query, top_k)
}

pub fn search_global_memories(query: &str, top_k: usize) -> Vec<String> {
    let home = match std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let global_mem_path = Path::new(&home).join(".freecode").join("global_memory.json");
    if !global_mem_path.exists() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&global_mem_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let items: Vec<MemoryItem> = match serde_json::from_str(&content) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };
    let doc_strings: Vec<String> = items.into_iter().map(|item| item.content).collect();
    rank_documents(&doc_strings, query, top_k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenization_and_stopwords() {
        let doc = "The quick brown fox jumps over the lazy dog!";
        let query = "lazy dog";
        let docs = vec![doc.to_string()];
        let results = rank_documents(&docs, query, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], doc);
    }

    #[test]
    fn test_bm25_ranking_relevance() {
        let doc1 = "We run all tests using the nextest framework for Rust.".to_string();
        let doc2 = "The database uses port 5432 for Postgres connections.".to_string();
        let doc3 = "Always add docstrings to public classes.".to_string();
        let docs = vec![doc1.clone(), doc2.clone(), doc3.clone()];

        // Query about tests should match doc1
        let results_tests = rank_documents(&docs, "how to run tests", 2);
        assert!(!results_tests.is_empty());
        assert_eq!(results_tests[0], doc1);

        // Query about database should match doc2
        let results_db = rank_documents(&docs, "Postgres port number", 2);
        assert!(!results_db.is_empty());
        assert_eq!(results_db[0], doc2);

        // Irrelevant query should return nothing
        let results_none = rank_documents(&docs, "completely unrelated string", 2);
        assert!(results_none.is_empty());
    }
}
