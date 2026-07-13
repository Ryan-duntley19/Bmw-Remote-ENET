# Installation Guide

Prefer the short path: **[QUICKSTART.md](QUICKSTART.md)**.

## First-time build (required for GitHub ZIP downloads)

The `installer\` folder does **not** ship `.exe` files in source form. Build them on Windows:

1. Install Rust from https://rustup.rs  
2. Open a **new** PowerShell in the repo root (or `installer\`)  
3. Run:

```powershell
.\installer\Build-Windows.ps1
```

This runs `cargo build --release` and copies:

- `enet-gateway.exe`, `enet-gui.exe`, `enet-setup.exe`, `enet-agent.exe`, `enet-relay.exe`

into `installer\`.

`Install-Desktop.bat` / `Install-Laptop.bat` will also attempt this build automatically if the `.exe` files are missing.

### "Hmmm... can't reach this page" / ERR_CONNECTION_REFUSED

That means **nothing is listening on port 47901** — usually because install printed
`enet-gateway.exe not found` and never started the service.

Fix: run `Build-Windows.ps1`, then re-run `Install-Desktop.bat` as Administrator.
Confirm the Windows service `BmwEnetGateway` is Running, or start:

```powershell
& "$env:ProgramFiles\BMW-ENET-Gateway\enet-gateway.exe" --config "$env:ProgramFiles\BMW-ENET-Gateway\config\gateway.toml"
```

Then open http://127.0.0.1:47901/ again.

---

## Desktop (double-click)

1. Build (see above) **or** copy into `installer/`:
   - `enet-gateway.exe`, `enet-gui.exe`, `enet-setup.exe`
2. Right-click **`Install-Desktop.bat`** → Run as administrator  
   (or `Setup-Gateway.ps1`)
3. Browser opens to the dashboard with your **pair code**.

The installer:

- Writes config under `Program Files\BMW-ENET-Gateway`
- Opens firewall for UDP **47900** (tunnel) + **47902** (discovery) on the local subnet
- Installs Windows service `BmwEnetGateway` (auto-start)
- Creates a desktop shortcut

## Laptop (double-click)

1. Build (see above) **or** copy into `installer/`: `enet-agent.exe`, `enet-setup.exe`
2. Run **`Install-Laptop.bat`** as administrator
3. Enter the pair code from the desktop (or press Enter to auto-find)

Install Npcap from https://npcap.com/ for real ENET capture.

## From source (any OS)

```bash
cargo build --release -p enet-setup -p enet-gateway -p enet-agent -p enet-sim

# Desktop
./target/release/enet-setup gateway --yes
./target/release/enet-gateway

# Laptop
./target/release/enet-setup agent
./target/release/enet-agent
```

Useful commands:

```bash
enet-setup find                 # list gateways on the LAN
enet-setup doctor --role gateway
enet-setup doctor --role agent
enet-agent --pair-code BMW-XXXX
```

## Virtual adapter (desktop tools)

Assign `169.254.1.1 / 255.255.0.0` on the `BMW-ENET` virtual NIC (Wintun/TAP).  
See [WINDOWS_DRIVERS.md](WINDOWS_DRIVERS.md).

## Uninstall

Run `installer/uninstall.bat` on each PC.
