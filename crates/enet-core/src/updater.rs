//! Self-update from GitHub Releases.
//!
//! Host and Client check the latest release, download their role package,
//! swap binaries in place (running exe is renamed to `.old` first), and
//! restart themselves. User config under `config/` is never touched.

use anyhow::{bail, Context};
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A newer release found on GitHub.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// Release tag (e.g. `v0.1.20`).
    pub tag: String,
    /// Parsed version (`0.1.20`).
    pub version: String,
    /// Direct download URL for the role asset.
    pub asset_url: String,
    /// Asset file name.
    pub asset_name: String,
    /// Release notes (truncated).
    pub notes: String,
    /// URL of the SHA256SUMS.txt asset when the release ships one.
    pub checksums_url: Option<String>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

fn parse_semver(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let mut it = v.split('.');
    let maj = it.next()?.parse().ok()?;
    let min = it.next()?.parse().ok()?;
    let pat: u64 = it
        .next()
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
        })
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    Some((maj, min, pat))
}

fn http_client() -> anyhow::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("bmw-enet-gateway/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()
        .context("build http client")
}

/// Check GitHub for a release newer than `current_version` that carries `asset_name`.
///
/// `token` is optional (needed while the repo is private).
pub fn check_latest(
    repo: &str,
    current_version: &str,
    asset_name: &str,
    token: &str,
) -> anyhow::Result<Option<UpdateInfo>> {
    let cur = parse_semver(current_version)
        .with_context(|| format!("bad current version {current_version}"))?;
    let client = http_client()?;
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut req = client
        .get(&url)
        .header("Accept", "application/vnd.github+json");
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req.send().context("query GitHub releases")?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND
        || resp.status() == reqwest::StatusCode::UNAUTHORIZED
        || resp.status() == reqwest::StatusCode::FORBIDDEN
    {
        // Private repo without token, rate limited, or no releases yet.
        bail!(
            "GitHub releases not accessible ({}). Repo private? Add update_token or make it public.",
            resp.status()
        );
    }
    let release: Release = resp.error_for_status()?.json().context("parse release")?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let latest = match parse_semver(&release.tag_name) {
        Some(v) => v,
        None => return Ok(None),
    };
    if latest <= cur {
        return Ok(None);
    }
    let asset = match release.assets.iter().find(|a| a.name == asset_name) {
        Some(a) => a,
        None => return Ok(None), // release exists but CI hasn't attached the asset yet
    };
    let pick_url = |a: &ReleaseAsset| {
        if token.is_empty() {
            a.browser_download_url.clone()
        } else {
            a.url.clone() // API asset endpoint (works on private repos)
        }
    };
    let asset_url = pick_url(asset);
    let checksums_url = release
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS.txt"))
        .map(pick_url);
    let mut notes = release.body.unwrap_or_default();
    notes.truncate(400);
    Ok(Some(UpdateInfo {
        version: release.tag_name.trim_start_matches('v').to_string(),
        tag: release.tag_name,
        asset_url,
        asset_name: asset_name.to_string(),
        notes,
        checksums_url,
    }))
}

/// Verify `bytes` against the release's SHA256SUMS.txt (when present).
fn verify_checksum(
    client: &reqwest::blocking::Client,
    info: &UpdateInfo,
    bytes: &[u8],
    token: &str,
) -> anyhow::Result<()> {
    let Some(url) = &info.checksums_url else {
        tracing::warn!("release has no SHA256SUMS.txt — skipping integrity check");
        return Ok(());
    };
    let mut req = client.get(url);
    if !token.is_empty() {
        req = req.bearer_auth(token).header("Accept", "application/octet-stream");
    }
    let sums = req
        .send()
        .context("download SHA256SUMS.txt")?
        .error_for_status()?
        .text()
        .context("read SHA256SUMS.txt")?;
    let expected = sums
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?;
            Some((name.trim_start_matches('*').to_string(), hash.to_lowercase()))
        })
        .find(|(name, _)| name.eq_ignore_ascii_case(&info.asset_name))
        .map(|(_, hash)| hash);
    let Some(expected) = expected else {
        bail!("SHA256SUMS.txt has no entry for {}", info.asset_name);
    };
    use sha2::{Digest, Sha256};
    let actual = hex::encode(Sha256::digest(bytes));
    if actual != expected {
        bail!(
            "checksum mismatch for {} (expected {expected}, got {actual}) — refusing to install",
            info.asset_name
        );
    }
    tracing::info!(asset = %info.asset_name, "update checksum verified");
    Ok(())
}

/// Download the update zip and swap files into `install_dir`.
///
/// `config/` and `logs/` entries in the zip are skipped so user settings survive.
/// Existing files (including the running exe) are renamed to `*.old` first.
pub fn download_and_stage(info: &UpdateInfo, install_dir: &Path, token: &str) -> anyhow::Result<Vec<String>> {
    let client = http_client()?;
    let mut req = client.get(&info.asset_url);
    if !token.is_empty() {
        req = req.bearer_auth(token).header("Accept", "application/octet-stream");
    }
    let bytes = req
        .send()
        .context("download update")?
        .error_for_status()?
        .bytes()
        .context("read update body")?;

    // Integrity check before anything touches the disk.
    verify_checksum(&client, info, bytes.as_ref(), token)?;

    let reader = std::io::Cursor::new(bytes.as_ref());
    let mut zip = zip::ZipArchive::new(reader).context("open update zip")?;

    // Sanity check BEFORE touching any file: the archive must contain our own
    // executable, otherwise this is the wrong asset and we'd brick the install.
    if let Some(exe_name) = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
    {
        let mut contains_exe = false;
        for i in 0..zip.len() {
            let entry = zip.by_index(i).context("scan zip entry")?;
            if let Some(rel) = entry.enclosed_name() {
                if rel
                    .file_name()
                    .map(|f| f.to_string_lossy().eq_ignore_ascii_case(&exe_name))
                    .unwrap_or(false)
                {
                    contains_exe = true;
                    break;
                }
            }
        }
        if !contains_exe {
            bail!("update zip does not contain {exe_name} — wrong asset, aborting");
        }
    }

    let mut updated = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).context("read zip entry")?;
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else { continue };
        let rel: PathBuf = rel.to_path_buf();
        let first = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if first == "config" || first == "logs" {
            continue; // never clobber user config or logs
        }
        let dest = install_dir.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf).context("read zip entry data")?;
        if dest.exists() {
            // Windows allows renaming a running exe, not overwriting it.
            let old = dest.with_extension(format!(
                "{}.old",
                dest.extension().and_then(|e| e.to_str()).unwrap_or("bin")
            ));
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&dest, &old)
                .with_context(|| format!("stage old file {}", dest.display()))?;
        }
        std::fs::write(&dest, &buf)
            .with_context(|| format!("write {}", dest.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if rel.extension().is_none() || rel.extension().and_then(|e| e.to_str()) == Some("exe") {
                let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
            }
        }
        updated.push(rel.display().to_string());
    }
    if updated.is_empty() {
        bail!("update zip contained no files to install");
    }
    Ok(updated)
}

/// Remove leftover `*.old` files from a previous update (call at startup).
pub fn cleanup_stale(install_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(install_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("old") {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

/// Relaunch the (freshly updated) current executable after a short delay and
/// exit this process. The delay lets sockets / single-instance locks release.
pub fn restart_self() -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("current exe")?;
    let args: Vec<String> = std::env::args().skip(1).collect();

    let workdir = exe
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let arg_str = args
            .iter()
            .map(|a| {
                if a.contains(' ') {
                    format!("\"{a}\"")
                } else {
                    a.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let cmdline = format!(
            "timeout /t 2 /nobreak >nul & start \"\" \"{}\" {}",
            exe.display(),
            arg_str
        );
        std::process::Command::new("cmd")
            .args(["/C", &cmdline])
            .current_dir(&workdir)
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn()
            .context("spawn restart helper")?;
    }
    #[cfg(unix)]
    {
        let arg_str = args
            .iter()
            .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join(" ");
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "sleep 2; exec '{}' {} >/dev/null 2>&1 &",
                exe.display(),
                arg_str
            ))
            .current_dir(&workdir)
            .spawn()
            .context("spawn restart helper")?;
    }

    tracing::info!("restarting for update");
    std::process::exit(0);
}

/// Install directory = folder containing the running executable.
pub fn install_dir() -> anyhow::Result<PathBuf> {
    Ok(std::env::current_exe()
        .context("current exe")?
        .parent()
        .context("exe has no parent dir")?
        .to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parses() {
        assert_eq!(parse_semver("v0.1.19"), Some((0, 1, 19)));
        assert_eq!(parse_semver("1.2.3"), Some((1, 2, 3)));
        assert!(parse_semver("v0.1.20") > parse_semver("v0.1.19"));
        assert!(parse_semver("nope").is_none());
    }
}
