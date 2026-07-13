//! Download BMW ENET packages from GitHub Releases (or use local offline files).

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Default GitHub repository that publishes Windows packages.
pub const DEFAULT_REPO: &str = "Ryan-duntley19/test";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Desktop PC that runs ISTA / E-Sys.
    Host,
    /// Laptop near the car with the ENET cable.
    Client,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Self::Host => "Host (Desktop)",
            Self::Client => "Client (Laptop)",
        }
    }

    pub fn asset_name(self) -> &'static str {
        match self {
            Self::Host => "BMW-ENET-Host-windows-x64.zip",
            Self::Client => "BMW-ENET-Client-windows-x64.zip",
        }
    }

    pub fn required_bins(self) -> &'static [&'static str] {
        match self {
            Self::Host => &["enet-gateway.exe", "enet-setup.exe"],
            Self::Client => &["enet-agent.exe", "enet-setup.exe"],
        }
    }

    pub fn optional_bins(self) -> &'static [&'static str] {
        match self {
            Self::Host => &["enet-gui.exe"],
            Self::Client => &[],
        }
    }

    pub fn install_dir_name(self) -> &'static str {
        match self {
            Self::Host => "BMW-ENET-Gateway",
            Self::Client => "BMW-ENET-Agent",
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn service_name(self) -> &'static str {
        match self {
            Self::Host => "BmwEnetGateway",
            Self::Client => "BmwEnetAgent",
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn service_display(self) -> &'static str {
        match self {
            Self::Host => "BMW ENET desktop gateway",
            Self::Client => "BMW ENET laptop agent",
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn main_exe(self) -> &'static str {
        match self {
            Self::Host => "enet-gateway.exe",
            Self::Client => "enet-agent.exe",
        }
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// Progress callback: (bytes_done, bytes_total_or_0, status message).
pub type ProgressFn = dyn Fn(u64, u64, &str) + Send + Sync;

pub struct PreparedPackage {
    #[allow(dead_code)]
    pub version: String,
    pub extract_dir: PathBuf,
}

/// Resolve package files: prefer offline files next to the Setup.exe, else download.
pub fn prepare_package(
    role: Role,
    repo: &str,
    setup_dir: &Path,
    work_dir: &Path,
    progress: &ProgressFn,
) -> Result<PreparedPackage> {
    fs::create_dir_all(work_dir)?;

    // 1) Offline: zip next to Setup.exe
    let local_zip = setup_dir.join(role.asset_name());
    if local_zip.is_file() {
        progress(0, 0, &format!("Using offline package {}", local_zip.display()));
        let extract_dir = work_dir.join("extract");
        if extract_dir.exists() {
            let _ = fs::remove_dir_all(&extract_dir);
        }
        fs::create_dir_all(&extract_dir)?;
        unzip_to(&local_zip, &extract_dir, progress)?;
        verify_bins(role, &extract_dir)?;
        return Ok(PreparedPackage {
            version: "offline".into(),
            extract_dir,
        });
    }

    // 2) Offline: binaries already next to Setup.exe (dev / copied build)
    if role
        .required_bins()
        .iter()
        .all(|b| setup_dir.join(b).is_file())
    {
        progress(0, 0, "Using binaries next to Setup.exe");
        let extract_dir = work_dir.join("extract");
        if extract_dir.exists() {
            let _ = fs::remove_dir_all(&extract_dir);
        }
        fs::create_dir_all(&extract_dir)?;
        for bin in role.required_bins().iter().chain(role.optional_bins().iter()) {
            let src = setup_dir.join(bin);
            if src.is_file() {
                fs::copy(&src, extract_dir.join(bin))?;
            }
        }
        return Ok(PreparedPackage {
            version: "local".into(),
            extract_dir,
        });
    }

    // 3) Download from GitHub Releases
    progress(0, 0, "Looking up latest GitHub release...");
    let client = reqwest::blocking::Client::builder()
        .user_agent("BMW-ENET-Setup")
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let api = format!("https://api.github.com/repos/{repo}/releases/latest");
    let release: Release = client
        .get(&api)
        .header("Accept", "application/vnd.github+json")
        .send()
        .with_context(|| format!("Failed to query {api}"))?
        .error_for_status()
        .context(
            "No GitHub release found yet. Publish a Windows release, or place the role zip next to BMW-ENET-Setup.exe",
        )?
        .json()
        .context("Invalid GitHub release JSON")?;

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == role.asset_name())
        .with_context(|| {
            format!(
                "Release {} has no asset named {}. Available: {}",
                release.tag_name,
                role.asset_name(),
                release
                    .assets
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    let zip_path = work_dir.join(role.asset_name());
    progress(
        0,
        asset.size,
        &format!(
            "Downloading {} ({:.1} MB)...",
            asset.name,
            asset.size as f64 / 1_048_576.0
        ),
    );
    download_file(&client, &asset.browser_download_url, &zip_path, asset.size, progress)?;

    let extract_dir = work_dir.join("extract");
    if extract_dir.exists() {
        let _ = fs::remove_dir_all(&extract_dir);
    }
    fs::create_dir_all(&extract_dir)?;
    unzip_to(&zip_path, &extract_dir, progress)?;
    verify_bins(role, &extract_dir)?;

    Ok(PreparedPackage {
        version: release.tag_name,
        extract_dir,
    })
}

fn download_file(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    expected_size: u64,
    progress: &ProgressFn,
) -> Result<()> {
    let mut resp = client
        .get(url)
        .send()
        .with_context(|| format!("Download failed: {url}"))?
        .error_for_status()?;

    let total = resp.content_length().unwrap_or(expected_size);
    let mut file = File::create(dest)?;
    let mut buf = [0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total, "Downloading...");
    }
    file.flush()?;
    Ok(())
}

fn unzip_to(zip_path: &Path, dest: &Path, progress: &ProgressFn) -> Result<()> {
    progress(0, 0, "Extracting package...");
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file
            .enclosed_name()
            .map(|p| p.to_owned())
            .ok_or_else(|| anyhow::anyhow!("unsafe zip path"))?;
        let out = dest.join(&name);
        if file.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = File::create(&out)?;
        io::copy(&mut file, &mut outfile)?;
    }
    // If zip contained a single top-level folder, flatten when bins are nested.
    flatten_if_needed(dest)?;
    Ok(())
}

fn flatten_if_needed(dest: &Path) -> Result<()> {
    let gateway = dest.join("enet-gateway.exe");
    let agent = dest.join("enet-agent.exe");
    if gateway.is_file() || agent.is_file() || dest.join("enet-setup.exe").is_file() {
        return Ok(());
    }
    for entry in fs::read_dir(dest)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let sub = entry.path();
            if sub.join("enet-setup.exe").is_file()
                || sub.join("enet-gateway.exe").is_file()
                || sub.join("enet-agent.exe").is_file()
            {
                for child in fs::read_dir(&sub)? {
                    let child = child?;
                    let target = dest.join(child.file_name());
                    let _ = fs::remove_file(&target);
                    let _ = fs::remove_dir_all(&target);
                    fs::rename(child.path(), &target)?;
                }
                let _ = fs::remove_dir_all(&sub);
                break;
            }
        }
    }
    Ok(())
}

fn verify_bins(role: Role, dir: &Path) -> Result<()> {
    for bin in role.required_bins() {
        if !dir.join(bin).is_file() {
            bail!("Package is missing required file: {bin}");
        }
    }
    Ok(())
}
