use std::process::Command;

pub const REPO: &str = "https://github.com/My-sidequests/Bullscript.git";

pub fn run() {
    println!("Updating Bullscript...");

    let remote = match remote_head(REPO, "main") {
        Some(h) => h,
        None => {
            eprintln!("Could not reach repository. Check your internet connection.");
            return;
        }
    };

    let installed = installed_hash("bullscript", REPO, "main");

	if installed.map_or(false, |h| h == remote) {
        println!("Already up to date (commit: {}).", &remote[..8]);
        return;
    }

    let status = Command::new("cargo")
        .args(["install", "--git", REPO, "--branch", "main", "--force", "bullscript"])
        .status();

    match status {
        Ok(s) if s.success() => println!("Update complete."),
        Ok(s)  => eprintln!("cargo install exited with {}.", s),
        Err(e) => eprintln!("Failed to run cargo: {}.", e),
    }
}

/// Fetch the HEAD commit hash of `branch` from a remote git repository.
/// Returns the full 40-character SHA, or None if git is unavailable or the
/// repo cannot be reached.
pub fn remote_head(repo: &str, branch: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["ls-remote", repo, &format!("refs/heads/{}", branch)])
        .output()
        .ok()?;

    let stdout = String::from_utf8(output.stdout).ok()?;
    let hash = stdout.split_whitespace().next()?;
    if hash.len() == 40 { Some(hash.to_string()) } else { None }
}

/// Read the commit hash for `package` as recorded in ~/.cargo/.crates2.json.
/// Returns the short hash stored by cargo (e.g. "aaec925f"), or None if not
/// found or the file cannot be parsed.
pub fn installed_hash(package: &str, repo: &str, branch: &str) -> Option<String> {
    let cargo_home = std::env::var("CARGO_installed_hashHOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            std::path::PathBuf::from(home).join(".cargo")
        });

    let content = std::fs::read_to_string(
        cargo_home.join(".crates2.json")
    ).ok()?;

    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let installs = json["installs"].as_object()?;

    let repo_fragment = repo.trim_end_matches(".git");
    let branch_tag = format!("branch={}", branch);

    for key in installs.keys() {
        if key.contains(package)
            && key.contains(repo_fragment)
            && key.contains(&branch_tag)
        {
            // key = "bullang 1.0.0 (git+...?branch=main#e61e4db6c4c8...)"
            let hash = key.split('#').nth(1)?.trim_end_matches(')');
            return Some(hash.to_string());
        }
    }
    None
}
