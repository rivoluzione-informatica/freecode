use std::time::{Instant, Duration};

pub fn run_git_cmd_with_retry(args: &[&str], cwd: &str) -> Result<String, String> {
    let mut last_err = String::new();
    for attempt in 1..=3 {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    return Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string());
                } else {
                    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    last_err = format!("Git command exit code failure (attempt {}): {}", attempt, err);
                }
            }
            Err(e) => {
                last_err = format!("Failed to execute Git command (attempt {}): {}", attempt, e);
            }
        }
        // Small backoff before retrying
        std::thread::sleep(Duration::from_millis(100 * attempt));
    }
    Err(last_err)
}

pub fn fetch_git_info_with_retry(workspace_path: &str) -> String {
    if run_git_cmd_with_retry(&["rev-parse", "--is-inside-work-tree"], workspace_path).is_err() {
        return "Not inside a Git repository or Git is not initialized.".to_string();
    }

    let mut git_info = String::new();

    // 1. Get branch info
    if let Ok(branch) = run_git_cmd_with_retry(&["rev-parse", "--abbrev-ref", "HEAD"], workspace_path) {
        git_info.push_str(&format!("* **Current Branch**: {}\n", branch));
    }

    // 2. Get status summary
    if let Ok(status) = run_git_cmd_with_retry(&["status", "--porcelain"], workspace_path) {
        if status.is_empty() {
            git_info.push_str("* **Status**: Working directory clean.\n");
        } else {
            git_info.push_str("* **Status** (uncommitted files):\n```\n");
            let limited_status: Vec<&str> = status.lines().take(20).collect();
            git_info.push_str(&limited_status.join("\n"));
            if status.lines().count() > 20 {
                git_info.push_str("\n... and more files");
            }
            git_info.push_str("\n```\n");
        }
    }

    // 3. Get recent commits
    if let Ok(log) = run_git_cmd_with_retry(&["log", "-n", "5", "--oneline"], workspace_path) {
        git_info.push_str("* **Recent Commits**:\n```\n");
        git_info.push_str(&log);
        git_info.push_str("\n```\n");
    }

    // 4. Get current diff
    if let Ok(diff) = run_git_cmd_with_retry(&["diff"], workspace_path) {
        if !diff.is_empty() {
            git_info.push_str("* **Uncommitted Git Diff**:\n```diff\n");
            let limited_diff: Vec<&str> = diff.lines().take(100).collect();
            git_info.push_str(&limited_diff.join("\n"));
            if diff.lines().count() > 100 {
                git_info.push_str("\n... [diff truncated for token limits]");
            }
            git_info.push_str("\n```\n");
        }
    }

    git_info
}

pub fn get_git_info_with_retry_and_cache(
    git_cache: &std::sync::Mutex<Option<(Instant, String)>>,
    workspace_path: &str,
) -> String {
    // Check cache first (Thundering herd protection)
    {
        let cache = git_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((instant, data)) = &*cache {
            if instant.elapsed() < Duration::from_secs(5) {
                println!("Returning cached git info (throttled).");
                return data.clone();
            }
        }
    }

    // Fetch and update cache
    println!("Fetching git info from repository...");
    let git_data = fetch_git_info_with_retry(workspace_path);
    {
        let mut cache = git_cache.lock().unwrap_or_else(|e| e.into_inner());
        *cache = Some((Instant::now(), git_data.clone()));
    }

    git_data
}

#[derive(Debug, Clone)]
pub struct GitStatusResult {
    pub is_inside_repo: bool,
    pub branch: String,
    pub modified_files: Vec<String>,
    pub added_files: Vec<String>,
    pub deleted_files: Vec<String>,
}

pub fn get_parsed_git_status(workspace_path: &str) -> GitStatusResult {
    if run_git_cmd_with_retry(&["rev-parse", "--is-inside-work-tree"], workspace_path).is_err() {
        return GitStatusResult {
            is_inside_repo: false,
            branch: String::new(),
            modified_files: Vec::new(),
            added_files: Vec::new(),
            deleted_files: Vec::new(),
        };
    }

    let branch = run_git_cmd_with_retry(&["rev-parse", "--abbrev-ref", "HEAD"], workspace_path)
        .unwrap_or_else(|_| "HEAD".to_string());

    let mut modified_files = Vec::new();
    let mut added_files = Vec::new();
    let mut deleted_files = Vec::new();

    if let Ok(status_out) = run_git_cmd_with_retry(&["status", "--porcelain"], workspace_path) {
        for line in status_out.lines() {
            if line.len() < 3 {
                continue;
            }
            let status_code = &line[0..2];
            let file_path = line[3..].trim().to_string();
            
            if status_code.contains('M') {
                modified_files.push(file_path);
            } else if status_code.contains('A') || status_code.contains('?') {
                added_files.push(file_path);
            } else if status_code.contains('D') {
                deleted_files.push(file_path);
            }
        }
    }

    GitStatusResult {
        is_inside_repo: true,
        branch,
        modified_files,
        added_files,
        deleted_files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;

    fn setup_git_repo(name: &str) -> std::path::PathBuf {
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let test_path = std::env::temp_dir().join(format!("freecode_test_git_{}_{}", name, unique_id));
        let _ = fs::create_dir_all(&test_path);
        
        let init_res = std::process::Command::new("git")
            .arg("init")
            .current_dir(&test_path)
            .output();
        if let Ok(out) = init_res {
            if !out.status.success() {
                println!("Warning: git init failed in test setup: {}", String::from_utf8_lossy(&out.stderr));
            }
        }
        
        let _ = std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&test_path)
            .output();
        let _ = std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&test_path)
            .output();

        test_path
    }

    #[test]
    fn test_git_status_detection() {
        let path = setup_git_repo("status");
        let path_str = path.to_string_lossy().to_string();

        let non_repo = std::env::temp_dir().join("freecode_non_existent_path_xyz");
        let status_non_repo = get_parsed_git_status(&non_repo.to_string_lossy());
        assert!(!status_non_repo.is_inside_repo);

        let file_path = path.join("file1.txt");
        File::create(&file_path).unwrap();

        let status1 = get_parsed_git_status(&path_str);
        assert!(status1.is_inside_repo);
        assert!(status1.added_files.contains(&"file1.txt".to_string()));

        let _ = std::process::Command::new("git").args(["add", "."]).current_dir(&path).output();
        let _ = std::process::Command::new("git").args(["commit", "-m", "initial"]).current_dir(&path).output();

        let status2 = get_parsed_git_status(&path_str);
        assert!(status2.modified_files.is_empty());
        assert!(status2.added_files.is_empty());
        assert!(status2.deleted_files.is_empty());

        {
            let mut f = File::create(&file_path).unwrap();
            write!(f, "modified content").unwrap();
        }

        let file_path_new = path.join("file2.txt");
        File::create(&file_path_new).unwrap();

        let status3 = get_parsed_git_status(&path_str);
        assert!(status3.modified_files.contains(&"file1.txt".to_string()));
        assert!(status3.added_files.contains(&"file2.txt".to_string()));

        let _ = fs::remove_dir_all(&path);
    }
}
