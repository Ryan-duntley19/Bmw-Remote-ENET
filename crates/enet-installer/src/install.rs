//! Windows install steps: copy files, firewall, auto-start, shortcuts.

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
    // Avoid "Program Files" spaces — they break sc.exe binPath and confuse users.
    #[cfg(windows)]
    {
        PathBuf::from(r"C:\BMW-ENET").join(match role {
            Role::Host => "Host",
            Role::Client => "Client",
        })
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
        // Remove any broken SCM service from older Setup builds (error 87).
        remove_legacy_scm_service(req.role, progress);
        if req.start_service {
            install_autostart(
                req.role,
                &install_dir,
                &config_path,
                &req.pair_code,
                progress,
            )?;
        }
        if req.role == Role::Host {
            create_desktop_shortcut(&install_dir, "BMW ENET Gateway", "http://127.0.0.1:47901/", progress)?;
        } else {
            create_desktop_shortcut(
                &install_dir,
                "BMW ENET Client Status",
                "http://127.0.0.1:47903/",
                progress,
            )?;
        }
        // Give the process a moment to bind the status port before opening the browser.
        if req.open_dashboard {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let url = if req.role == Role::Host {
                "http://127.0.0.1:47901/"
            } else {
                "http://127.0.0.1:47903/"
            };
            let _ = Command::new("cmd")
                .args(["/C", "start", "", url])
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
                Some("http://127.0.0.1:47903/".into())
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
api_port = 47903
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
                "New-NetFirewallRule -DisplayName 'BMW ENET Tunnel' -Direction Inbound -Protocol UDP -LocalPort 47900 -Action Allow -Profile Any | Out-Null; New-NetFirewallRule -DisplayName 'BMW ENET Discovery' -Direction Inbound -Protocol UDP -LocalPort 47902 -Action Allow -Profile Any | Out-Null",
            ])
            .status();
    }
    Ok(())
}

#[cfg(windows)]
fn remove_legacy_scm_service(role: Role, progress: &ProgressFn) {
    let name = role.service_name();
    let _ = Command::new("sc.exe").args(["stop", name]).output();
    let del = Command::new("sc.exe").args(["delete", name]).output();
    if del
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("SUCCESS"))
        .unwrap_or(false)
    {
        progress(0, 0, &format!("Removed old broken Windows service {name}"));
    }
}

#[cfg(windows)]
fn install_autostart(
    role: Role,
    install_dir: &Path,
    config_path: &Path,
    pair_code: &str,
    progress: &ProgressFn,
) -> Result<()> {
    progress(0, 0, "Configuring auto-start + launching now...");
    let exe = install_dir.join(role.main_exe());
    if !exe.is_file() {
        bail!("Missing {}", exe.display());
    }

    let mut args = format!("--config \"{}\"", config_path.display());
    if role == Role::Client && !pair_code.trim().is_empty() {
        args.push_str(&format!(" --pair-code {}", pair_code.trim()));
    }

    let task_name = match role {
        Role::Host => "BMW-ENET-Host",
        Role::Client => "BMW-ENET-Client",
    };

    // Scheduled Task at startup (works for normal console apps; SCM services need a service main).
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$taskName = '{task}'
Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue | Out-Null
$action = New-ScheduledTaskAction -Execute '{exe}' -Argument '{args}' -WorkingDirectory '{wd}'
$trigger = New-ScheduledTaskTrigger -AtStartup
$principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit ([TimeSpan]::Zero) -StartWhenAvailable
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
Start-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
"#,
        task = task_name,
        exe = exe.display(),
        args = args.replace('\'', "''"),
        wd = install_dir.display(),
    );

    let status = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .status()
        .context("register scheduled task")?;
    if !status.success() {
        progress(
            0,
            0,
            "Scheduled task registration failed — will still try to start the app now",
        );
    } else {
        progress(0, 0, &format!("Auto-start task registered: {task_name}"));
    }

    // Start immediately (do not rely only on the task).
    start_now(&exe, config_path, role, pair_code, install_dir, progress)?;
    Ok(())
}

#[cfg(windows)]
fn start_now(
    exe: &Path,
    config_path: &Path,
    role: Role,
    pair_code: &str,
    install_dir: &Path,
    progress: &ProgressFn,
) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    let mut cmd = Command::new(exe);
    cmd.arg("--config")
        .arg(config_path)
        .current_dir(install_dir)
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    if role == Role::Client && !pair_code.trim().is_empty() {
        cmd.arg("--pair-code").arg(pair_code.trim());
    }
    match cmd.spawn() {
        Ok(_) => {
            progress(0, 0, &format!("{} started", role.main_exe()));
            Ok(())
        }
        Err(e) => {
            bail!("Failed to start {}: {e}", exe.display());
        }
    }
}

#[cfg(windows)]
fn create_desktop_shortcut(
    install_dir: &Path,
    title: &str,
    url: &str,
    progress: &ProgressFn,
) -> Result<()> {
    let gui = install_dir.join("enet-gui.exe");
    progress(0, 0, &format!("Creating desktop shortcut ({title})..."));
    let (target_path, args, workdir) = if gui.is_file() && title.contains("Gateway") {
        (
            gui.display().to_string(),
            String::new(),
            install_dir.display().to_string(),
        )
    } else {
        (
            r"C:\Windows\System32\cmd.exe".into(),
            format!("/C start {url}"),
            install_dir.display().to_string(),
        )
    };
    let script = format!(
        r#"
$desktop = [Environment]::GetFolderPath('Desktop')
$lnkPath = Join-Path $desktop '{title}.lnk'
$w = New-Object -ComObject WScript.Shell
$s = $w.CreateShortcut($lnkPath)
$s.TargetPath = '{target}'
$s.Arguments = '{args}'
$s.WorkingDirectory = '{wd}'
$s.Description = '{title}'
$s.Save()
"#,
        title = title.replace('\'', "''"),
        target = target_path.replace('\'', "''"),
        args = args.replace('\'', "''"),
        wd = workdir.replace('\'', "''"),
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status();
    Ok(())
}
