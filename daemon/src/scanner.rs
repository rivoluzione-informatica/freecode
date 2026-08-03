use std::time::Instant;

pub fn is_excluded(rel_path: &str, patterns: &[String]) -> bool {
    let normalized = rel_path.replace('\\', "/");
    for pat in patterns {
        let pat_clean = pat.replace('*', "");
        if pat.starts_with('*') && pat.ends_with('*') {
            if normalized.contains(&pat_clean) { return true; }
        } else if pat.starts_with('*') {
            if normalized.ends_with(&pat_clean) { return true; }
        } else if pat.ends_with('*') {
            if normalized.starts_with(&pat_clean) { return true; }
        } else {
            // No wildcard: match the exact path, a directory prefix (`pat/...`),
            // or a full path segment — but NOT a substring (so "src" doesn't
            // exclude "mysrc/x" or "src_old").
            let is_dir_prefix = normalized.starts_with(&format!("{}/", pat));
            let is_segment = normalized.split('/').any(|seg| seg == pat);
            if normalized == *pat || is_dir_prefix || is_segment {
                return true;
            }
        }
    }
    false
}

pub fn build_file_tree(
    root: &std::path::Path,
    current: &std::path::Path,
    depth: usize,
    lines: &mut Vec<String>,
    patterns: &[String],
) {
    if depth > 4 {
        return;
    }

    if let Ok(entries) = std::fs::read_dir(current) {
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| (e.file_type().map(|t| !t.is_dir()).unwrap_or(true), e.file_name()));

        for entry in entries {
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();
            
            if file_name_str.starts_with('.') 
                || file_name_str == "target"
                || file_name_str == "node_modules"
                || file_name_str == "dist"
                || file_name_str == "build"
                || file_name_str == "Cargo.lock"
                || file_name_str == "package-lock.json"
            {
                continue;
            }

            let path = entry.path();
            let rel_path = path.strip_prefix(root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| file_name_str.clone().into_owned());
            if is_excluded(&rel_path, patterns) {
                continue;
            }

            let indent = "  ".repeat(depth);
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                lines.push(format!("{}- {}/", indent, file_name_str));
                build_file_tree(root, &entry.path(), depth + 1, lines, patterns);
            } else {
                lines.push(format!("{}- {}", indent, file_name_str));
            }
        }
    }
}

/// The four symbol-extraction regexes, compiled once per scan and threaded down the
/// recursion as one value (they used to be four separate parameters — 9 args in total).
pub struct SymbolRegexes {
    pub fn_rs: regex::Regex,
    pub struct_rs: regex::Regex,
    pub fn_ts: regex::Regex,
    pub class_ts: regex::Regex,
}

impl Default for SymbolRegexes {
    fn default() -> Self {
        // Literals are fixed and known-valid — `unwrap` here can only fire on a source edit,
        // and every call site already assumed that.
        SymbolRegexes {
            fn_rs: regex::Regex::new(r#"(?:pub\s+)?(?:async\s+)?fn\s+([a-zA-Z0-9_]+)"#).unwrap(),
            struct_rs: regex::Regex::new(r#"(?:pub\s+)?struct\s+([a-zA-Z0-9_]+)"#).unwrap(),
            fn_ts: regex::Regex::new(r#"(?:export\s+)?(?:async\s+)?function\s+([a-zA-Z0-9_]+)"#).unwrap(),
            class_ts: regex::Regex::new(r#"(?:export\s+)?class\s+([a-zA-Z0-9_]+)"#).unwrap(),
        }
    }
}

pub fn find_major_symbols_recursive(
    root: &std::path::Path,
    current: &std::path::Path,
    symbols: &mut Vec<String>,
    files_read: &mut Vec<String>,
    res: &SymbolRegexes,
    patterns: &[String],
) {
    if let Ok(entries) = std::fs::read_dir(current) {
        for entry in entries.filter_map(Result::ok) {
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();
            
            if file_name_str.starts_with('.') 
                || file_name_str == "target"
                || file_name_str == "node_modules"
                || file_name_str == "dist"
                || file_name_str == "build"
            {
                continue;
            }

            let path = entry.path();
            let rel_path = path.strip_prefix(root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| file_name_str.clone().into_owned());

            if is_excluded(&rel_path, patterns) {
                continue;
            }

            if path.is_dir() {
                find_major_symbols_recursive(root, &path, symbols, files_read, res, patterns);
            } else {
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                if ext == "rs" || ext == "ts" || ext == "js" {
                    if let Ok(content) = std::fs::read_to_string(&path) {

                        files_read.push(rel_path.clone());
                        let mut file_symbols = Vec::new();

                        if ext == "rs" {
                            for cap in res.fn_rs.captures_iter(&content) {
                                file_symbols.push(format!("  - fn {}", &cap[1]));
                            }
                            for cap in res.struct_rs.captures_iter(&content) {
                                file_symbols.push(format!("  - struct {}", &cap[1]));
                            }
                        } else {
                            for cap in res.fn_ts.captures_iter(&content) {
                                file_symbols.push(format!("  - function {}", &cap[1]));
                            }
                            for cap in res.class_ts.captures_iter(&content) {
                                file_symbols.push(format!("  - class {}", &cap[1]));
                            }
                        }

                        if !file_symbols.is_empty() {
                            let count = file_symbols.len();
                            let symbols_taken: Vec<String> = file_symbols.into_iter().take(10).collect();
                            let mut line = format!("* **{}**:\n{}", rel_path, symbols_taken.join("\n"));
                            if count > 10 {
                                line.push_str(&format!("\n  - ... and {} more", count - 10));
                            }
                            symbols.push(line);
                        }
                    }
                }
            }
        }
    }
}

pub fn find_major_symbols(
    root: &std::path::Path,
    current: &std::path::Path,
    symbols: &mut Vec<String>,
    files_read: &mut Vec<String>,
    patterns: &[String],
) {
    let res = SymbolRegexes::default();
    find_major_symbols_recursive(root, current, symbols, files_read, &res, patterns);
}

pub fn generate_workspace_overview(
    git_cache: &std::sync::Mutex<Option<(Instant, String)>>,
    workspace_path: &str,
    patterns: &[String],
) -> (String, Vec<String>) {
    let mut overview = String::new();
    let mut files_read = Vec::new();
    let path = std::path::Path::new(workspace_path);
    if !path.exists() {
        return ("Workspace path does not exist.".to_string(), files_read);
    }

    overview.push_str(&format!("Active Workspace Path: {}\n\n", workspace_path));

    // 1. Fetch Git Info (with retry + cache)
    overview.push_str("### Git Repository Context\n");
    let git_info = crate::git::get_git_info_with_retry_and_cache(git_cache, workspace_path);
    overview.push_str(&git_info);
    overview.push('\n');

    let git_head = path.join(".git").join("HEAD");
    if git_head.exists() {
        files_read.push(".git/HEAD".to_string());
    }

    // 2. Traverse and build file tree
    overview.push_str("### Directory Structure\n```\n");
    let mut tree_lines = Vec::new();
    build_file_tree(path, path, 0, &mut tree_lines, patterns);
    overview.push_str(&tree_lines.join("\n"));
    overview.push_str("\n```\n\n");

    // 3. Identify and print project config files
    overview.push_str("### Key Project Files & Configuration\n");
    
    // Check Cargo.toml
    let cargo_toml = path.join("Cargo.toml");
    if cargo_toml.exists() {
        if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
            files_read.push("Cargo.toml".to_string());
            overview.push_str("#### Cargo.toml (Rust Project Configuration)\n```toml\n");
            let lines: Vec<&str> = content.lines().take(40).collect();
            overview.push_str(&lines.join("\n"));
            overview.push_str("\n```\n\n");
        }
    }

    // Check package.json
    let package_json = path.join("package.json");
    if package_json.exists() {
        if let Ok(content) = std::fs::read_to_string(&package_json) {
            files_read.push("package.json".to_string());
            overview.push_str("#### package.json (Node/JS/TS Project Configuration)\n```json\n");
            let lines: Vec<&str> = content.lines().take(40).collect();
            overview.push_str(&lines.join("\n"));
            overview.push_str("\n```\n\n");
        }
    }

    // 4. Extract major code symbols
    overview.push_str("### Key Code Symbols & Functions\n");
    let mut symbols_list = Vec::new();
    find_major_symbols(path, path, &mut symbols_list, &mut files_read, patterns);
    if symbols_list.is_empty() {
        overview.push_str("No major symbols found.\n");
    } else {
        overview.push_str(&symbols_list.join("\n"));
    }
    overview.push('\n');

    (overview, files_read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;

    #[test]
    fn test_is_excluded() {
        let pats = vec!["src".to_string()];
        // exact + dir-prefix + segment match
        assert!(is_excluded("src", &pats));
        assert!(is_excluded("src/main.rs", &pats));
        assert!(is_excluded("a/src/b.rs", &pats));
        // NOT a substring match
        assert!(!is_excluded("mysrc/x.rs", &pats));
        assert!(!is_excluded("src_old/x.rs", &pats));

        // wildcards still work
        assert!(is_excluded("debug.log", &["*.log".to_string()]));
        assert!(is_excluded("dist/bundle.js", &["dist*".to_string()]));
        assert!(is_excluded("a/node_modules/b", &["*node_modules*".to_string()]));
        assert!(!is_excluded("README.md", &["*.log".to_string()]));
    }

    fn setup_test_dir(name: &str) -> std::path::PathBuf {
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_path = std::env::temp_dir().join(format!("freecode_test_{}_{}", name, unique_id));
        let _ = fs::create_dir_all(&test_path);
        test_path
    }

    #[test]
    fn test_build_file_tree() {
        let path = setup_test_dir("file_tree");

        let sub_dir = path.join("subdir");
        fs::create_dir(&sub_dir).unwrap();
        
        File::create(path.join("file1.txt")).unwrap();
        File::create(sub_dir.join("file2.rs")).unwrap();
        
        // Excluded files
        File::create(path.join(".git")).unwrap();
        let node_modules = path.join("node_modules");
        fs::create_dir(&node_modules).unwrap();
        File::create(node_modules.join("dep.js")).unwrap();

        let mut lines = Vec::new();
        build_file_tree(&path, &path, 0, &mut lines, &[]);

        let lines_str = lines.join("\n");
        assert!(lines_str.contains("- file1.txt"));
        assert!(lines_str.contains("- subdir/"));
        assert!(lines_str.contains("  - file2.rs"));
        assert!(!lines_str.contains(".git"));
        assert!(!lines_str.contains("node_modules"));

        // Clean up
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn test_find_major_symbols() {
        let path = setup_test_dir("symbols");

        let rs_content = r#"
            pub async fn calculate_sum(a: i32, b: i32) -> i32 { a + b }
            struct ConfigState { port: u16 }
            fn helper() {}
        "#;
        
        let ts_content = r#"
            export async function startApp() {}
            export class ServerManager {}
            function localHelper() {}
        "#;

        let mut file1 = File::create(path.join("main.rs")).unwrap();
        write!(file1, "{}", rs_content).unwrap();

        let mut file2 = File::create(path.join("index.ts")).unwrap();
        write!(file2, "{}", ts_content).unwrap();

        let mut symbols = Vec::new();
        find_major_symbols(&path, &path, &mut symbols, &mut Vec::new(), &[]);

        let symbols_str = symbols.join("\n");
        assert!(symbols_str.contains("fn calculate_sum"));
        assert!(symbols_str.contains("struct ConfigState"));
        assert!(symbols_str.contains("fn helper"));
        assert!(symbols_str.contains("function startApp"));
        assert!(symbols_str.contains("class ServerManager"));
        assert!(symbols_str.contains("function localHelper"));

        // Clean up
        let _ = fs::remove_dir_all(&path);
    }
}
