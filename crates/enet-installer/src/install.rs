//! Windows install steps: copy files, firewall, service, config, shortcuts.

use crate::download::{PreparedPackage, Role};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub type ProgressFn = dyn Fn(u64, u64, &str) + Send + Sync;

pub struct InstallRequest {
    pub role: Role,
    pub pair_code: String,
    pub password: String,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub start_service: bool,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub open_dashboard: bool,
}

pub struct InstallResult {
    pub install_dir: PathBuf,
    pub version: String,
    pub pair_code_hint: String,
    pub dashboard_url: Option<String>,
}

pub fn default_install_dir(role: Role) -> PathBuf {
    #[cfg(windows)]
    {
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        PathBuf::from(pf).join(role.install_dir_name())
    }
    #[cfg(not(windows))]
    {
        std::env::temp_dir().join(role.install_dir_name())
    }
}

pub fn run_install(
    req: &InstallRequest,
    package: &PreparedPackage,
    progress: &ProgressFn,
) -> Result<InstallResult> {
    let install_dir = default_install_dir(req.role);
    progress(0, 0, &format!("Installing to {}...", install_dir.display()));

    fs::create_dir_all(install_dir.join("config"))?;
    fs::create_dir_all(install_dir.join("logs"))?;

    for bin in req
        .role
        .required_bins()
        .iter()
        .chain(req.role.optional_bins().iter())
    {
        let src = package.extract_dir.join(bin);
        if src.is_file() {
            fs::copy(&src, install_dir.join(bin))
                .with_context(|| format!("Copy {bin}"))?;
            progress(0, 0, &format!("Copied {bin}"));
        }
    }

    let config_path = install_dir.join("config").join(match req.role {
        Role::Host => "gateway.toml",
        Role::Client => "agent.toml",
    });

    write_config(req, &config_path)?;
    progress(0, 0, "Wrote configuration");

    // Prefer enet-setup when present for pair-code generation / validation.
    let setup = install_dir.join("enet-setup.exe");
    if setup.is_file() {
        let mut args: Vec<String> = match req.role {
            Role::Host => vec![
                "gateway".into(),
                "--config".into(),
                config_path.display().to_string(),
                "--yes".into(),
            ],
            Role::Client => {
                let mut a = vec![
                    "agent".into(),
                    "--config".into(),
                    config_path.display().to_string(),
                    "--yes".into(),
                ];
                if !req.pair_code.trim().is_empty() {
                    a.push("--pair-code".into());
                    a.push(req.pair_code.trim().to_string());
                }
                a
            }
        };
        if !req.password.is_empty() {
            args.push("--password".into());
            args.push(req.password.clone());
        }
        progress(0, 0, "Running enet-setup...");
        let status = Command::new(&setup).args(&args).status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => progress(0, 0, &format!("enet-setup exited with {s} (config already written)")),
            Err(e) => progress(0, 0, &format!("enet-setup launch note: {e}")),
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (progress, package, req);
        bail!(
            "This setup wizard installs on Windows only. On Linux/macOS use cargo run -p enet-gateway / enet-agent."
        );
    }

    #[cfg(windows)]
    {
        configure_firewall(req.role, progress)?;
        if req.start_service {
            install_and_start_service(
                req.role,
                &install_dir,
                &config_path,
                &req.pair_code,
                progress,
            )?;
        }
        if req.role == Role::Host {
            create_desktop_shortcut(&install_dir, progress)?;
        }
        if req.open_dashboard && req.role == Role::Host {
            let _ = Command::new("cmd")
                .args(["/C", "start", "", "http://127.0.0.1:47901/"])
                .status();
        }

        let pair_hint = read_pair_code(&config_path).unwrap_or_else(|| req.pair_code.clone());

        Ok(InstallResult {
            install_dir,
            version: package.version.clone(),
            pair_code_hint: pair_hint,
            dashboard_url: if req.role == Role::Host {
                Some("http://127.0.0.1:47901/".into())
            } else {
                None
            },
        })
    }
}

fn write_config(req: &InstallRequest, path: &Path) -> Result<()> {
    let password_line = if req.password.is_empty() {
        r#"password = """#.to_string()
    } else {
        format!(r#"password = "{}""#, req.password.replace('"', ""))
    };

    let contents = match req.role {
        Role::Host => format!(
            r#"# Generated by BMW-ENET-Setup
role = "gateway"
network_mode = "lan"
tunnel_port = 47900
api_port = 47901
discovery_port = 47902
auto_discover = true
pair_code = ""
{password_line}
require_crypto = false
relay_url = ""
auto_start = true
setup_complete = false
virtual_interface = "BMW-ENET"
tester_ip = "169.254.1.1"
tester_mask = "255.255.0.0"
log_level = "info"
log_dir = "logs"
manage_firewall = true
"#
        ),
        Role::Client => format!(
            r#"# Generated by BMW-ENET-Setup
role = "agent"
network_mode = "lan"
tunnel_port = 47900
api_port = 47901
discovery_port = 47902
auto_discover = true
pair_code = "{}"
{password_line}
require_crypto = false
relay_url = ""
auto_start = true
setup_complete = false
log_level = "info"
log_dir = "logs"
manage_firewall = true
"#,
            req.pair_code.trim().replace('"', "")
        ),
    };
    fs::write(path, contents)?;
    Ok(())
}

#[cfg_attr(not(windows), allow(dead_code))]
fn read_pair_code(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("pair_code") {
            let rest = rest.trim().trim_start_matches('=').trim();
            let v = rest.trim_matches('"').trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

#[cfg(windows)]
fn configure_firewall(role: Role, progress: &ProgressFn) -> Result<()> {
    progress(0, 0, "Configuring Windows Firewall...");
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-NetFirewallRule -DisplayName 'BMW ENET Tunnel' -ErrorAction SilentlyContinue | Remove-NetFirewallRule; Get-NetFirewallRule -DisplayName 'BMW ENET Discovery' -ErrorAction SilentlyContinue | Remove-NetFirewallRule",
        ])
        .status();

    if role == Role::Host {
        let _ = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "New-NetFirewallRule -DisplayName 'BMW ENET Tunnel' -Direction Inbound -Protocol UDP -LocalPort 47900 -RemoteAddress LocalSubnet -Action Allow -Profile Private | Out-Null; New-NetFirewallRule -DisplayName 'BMW ENET Discovery' -Direction Inbound -Protocol UDP -LocalPort 47902 -RemoteAddress LocalSubnet -Action Allow -Profile Private | Out-Null",
            ])
            .status();
    }
    Ok(())
}

#[cfg(windows)]
fn install_and_start_service(
    role: Role,
    install_dir: &Path,
    config_path: &Path,
    pair_code: &str,
    progress: &ProgressFn,
) -> Result<()> {
    progress(0, 0, "Installing Windows service...");
    let name = role.service_name();
    let exe = install_dir.join(role.main_exe());
    let mut bin_path = format!("\"{}\" --config \"{}\"", exe.display(), config_path.display());
    if role == Role::Client && !pair_code.trim().is_empty() {
        bin_path.push_str(&format!(" --pair-code {}", pair_code.trim()));
    }

    let _ = Command::new("sc.exe").args(["stop", name]).status();
    let _ = Command::new("sc.exe").args(["delete", name]).status();

    let create = Command::new("sc.exe")
        .args(["create", name, &format!("binPath= {bin_path}"), "start=", "auto"])
        .status()
        .context("sc create")?;
    if !create.success() {
        bail!("Failed to create service {name}. Try running Setup as Administrator.");
    }
    let _ = Command::new("sc.exe")
        .args(["description", name, role.service_display()])
        .status();
    let start = Command::new("sc.exe").args(["start", name]).status()?;
    if start.success() {
        progress(0, 0, &format!("Service {name} started"));
    } else {
        progress(
            0,
            0,
            &format!("Service created but not started — run: sc start {name}"),
        );
    }
    Ok(())
}

#[cfg(windows)]
fn create_desktop_shortcut(install_dir: &Path, progress: &ProgressFn) -> Result<()> {
    let gui = install_dir.join("enet-gui.exe");
    if !gui.is_file() {
        return Ok(());
    }
    progress(0, 0, "Creating desktop shortcut...");
    let script = format!(
        r#"
$desktop = [Environment]::GetFolderPath('Desktop')
$lnkPath = Join-Path $desktop 'BMW ENET Gateway.lnk'
$w = New-Object -ComObject WScript.Shell
$s = $w.CreateShortcut($lnkPath)
$s.TargetPath = '{}'
$s.WorkingDirectory = '{}'
$s.Description = 'BMW ENET Gateway dashboard'
$s.Save()
"#,
        gui.display(),
        install_dir.display()
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status();
    Ok(())
}
