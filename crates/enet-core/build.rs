use std::process::Command;

fn main() {
    let repo = std::env::var("BMW_ENET_GITHUB_REPO")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("GITHUB_REPOSITORY")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(repo_from_git_remote)
        .unwrap_or_default();

    println!("cargo:rerun-if-env-changed=BMW_ENET_GITHUB_REPO");
    println!("cargo:rerun-if-env-changed=GITHUB_REPOSITORY");
    println!("cargo:rustc-env=BMW_ENET_GITHUB_REPO={repo}");
}

fn repo_from_git_remote() -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout);
    parse_github_slug(url.trim())
}

fn parse_github_slug(raw: &str) -> Option<String> {
    let s = raw.trim().trim_end_matches('/').trim_end_matches(".git");
    let after = if let Some(i) = s.find("github.com/") {
        &s[i + "github.com/".len()..]
    } else if let Some(i) = s.find("github.com:") {
        &s[i + "github.com:".len()..]
    } else {
        return None;
    };
    let parts: Vec<_> = after.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 {
        Some(format!("{}/{}", parts[0], parts[1]))
    } else {
        None
    }
}
