# Installation Guide

**End users (Windows):** use the setup wizard — **[SETUP_WIZARD.md](SETUP_WIZARD.md)**  
Download `BMW-ENET-Setup.exe`, choose **Host** or **Client**. No Rust required.

---

## Developer / from-source build

Prefer the short path: **[QUICKSTART.md](QUICKSTART.md)**.

## First-time build (developers only)

```powershell
# Requires Rust: https://rustup.rs
.\installer\Build-Windows.ps1
```

This builds `BMW-ENET-Setup.exe` plus the role binaries and copies them into `installer\`.

CI publishes the same artifacts via `.github/workflows/windows-installer.yml`.

### "Hmmm... can't reach this page" / ERR_CONNECTION_REFUSED

That means **nothing is listening on port 47901** — usually because Host was never installed.

Fix: run **`BMW-ENET-Setup.exe`** as Administrator → **Host (Desktop)**.  
Confirm service `BmwEnetGateway` is Running, then open http://127.0.0.1:47901/ again.

---

## Legacy bat installers (optional)

Still available under `installer\` for advanced use:

1. Build (see above) **or** copy into `installer/`:
   - `enet-gateway.exe`, `enet-gui.exe`, `enet-setup.exe`
2. Right-click **`Install-Desktop.bat`** → Run as administrator
3. Browser opens to the dashboard with your **pair code**.

## Laptop (legacy)

1. Build **or** copy into `installer/`: `enet-agent.exe`, `enet-setup.exe`
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
