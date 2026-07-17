//! Npcap/WinPcap detection and interactive installer launch.
//!
//! Free Npcap does **not** support silent `/S` install (OEM only). We download
//! the official installer and run it interactively so the user can enable
//! “WinPcap API-compatible Mode”.

use anyhow::Result;

/// Official free-edition installer (bump when Npcap ships a new release).
pub const NPCAP_INSTALLER_URL: &str = "https://npcap.com/dist/npcap-1.88.exe";
/// Filename used under the system temp directory.
pub const NPCAP_INSTALLER_NAME: &str = "npcap-1.88.exe";

/// True when `wpcap.dll` is present (Npcap or legacy WinPcap).
pub fn npcap_installed() -> bool {
    #[cfg(windows)]
    {
        use std::path::Path;
        Path::new(r"C:\Windows\System32\Npcap\wpcap.dll").is_file()
            || Path::new(r"C:\Windows\System32\wpcap.dll").is_file()
            || Path::new(r"C:\Windows\SysWOW64\Npcap\wpcap.dll").is_file()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// If Npcap is missing, download the official installer and launch it.
///
/// Returns `Ok(true)` when Npcap is present afterward, `Ok(false)` when the
/// installer ran (or was opened) but DLLs are still missing.
pub fn ensure_npcap_installed(mut progress: impl FnMut(&str)) -> Result<bool> {
    if npcap_installed() {
        progress("Npcap found — L2 capture/inject available");
        return Ok(true);
    }

    #[cfg(not(windows))]
    {
        progress("Npcap is Windows-only");
        return Ok(false);
    }

    #[cfg(windows)]
    {
        use anyhow::Context;
        use std::time::Duration;
        use tracing::info;

        progress("Npcap missing — downloading installer from npcap.com…");
        let path = match download_npcap_installer(&mut progress) {
            Ok(p) => p,
            Err(e) => {
                progress(&format!(
                    "Download failed ({e:#}). Opening https://npcap.com/#download …"
                ));
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", "", "https://npcap.com/#download"])
                    .status();
                return Ok(false);
            }
        };

        progress(
            "Launching Npcap installer — enable “WinPcap API-compatible Mode”, then Finish…",
        );
        eprintln!();
        eprintln!("  *** Npcap installer ***");
        eprintln!("  Check: WinPcap API-compatible Mode");
        eprintln!("  Leave other defaults, then Finish.");
        eprintln!();

        let status = std::process::Command::new(&path)
            .status()
            .with_context(|| format!("launch {}", path.display()))?;
        info!(?status, "Npcap installer exited");

        // Installer may need a moment to unlock / register DLLs.
        for _ in 0..15 {
            if npcap_installed() {
                progress("Npcap installed successfully");
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(400));
        }

        if npcap_installed() {
            progress("Npcap installed successfully");
            Ok(true)
        } else {
            progress(
                "Npcap still not detected — install with WinPcap mode, then restart BMW ENET",
            );
            Ok(false)
        }
    }
}

#[cfg(windows)]
fn download_npcap_installer(progress: &mut impl FnMut(&str)) -> Result<std::path::PathBuf> {
    use anyhow::{bail, Context};
    use std::io::{Read, Write};
    use std::time::Duration;

    let dir = std::env::temp_dir().join("bmw-enet-npcap");
    std::fs::create_dir_all(&dir).context("create temp dir")?;
    let dest = dir.join(NPCAP_INSTALLER_NAME);

    // Reuse a recent download if present and non-trivial size.
    if dest.is_file() {
        if let Ok(meta) = dest.metadata() {
            if meta.len() > 1_000_000 {
                progress("Using cached Npcap installer");
                return Ok(dest);
            }
        }
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(format!(
            "BMW-ENET-Gateway/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .context("http client")?;

    progress(&format!("GET {NPCAP_INSTALLER_URL}"));
    let mut resp = client
        .get(NPCAP_INSTALLER_URL)
        .send()
        .context("download Npcap")?
        .error_for_status()
        .context("Npcap download HTTP error")?;

    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&dest).context("create installer file")?;
    let mut downloaded = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = resp.read(&mut buf).context("read download")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).context("write installer")?;
        downloaded += n as u64;
        if total > 0 && downloaded % (512 * 1024) < n as u64 {
            progress(&format!(
                "Downloading Npcap… {} / {} KB",
                downloaded / 1024,
                total / 1024
            ));
        }
    }
    file.flush().ok();

    if downloaded < 1_000_000 {
        let _ = std::fs::remove_file(&dest);
        bail!("Npcap download too small ({downloaded} bytes) — check network");
    }
    progress(&format!(
        "Downloaded Npcap installer ({} KB)",
        downloaded / 1024
    ));
    Ok(dest)
}
