# Quick Start (5 minutes)

## Important: source zip has no `.exe` files

If you downloaded this repo as a ZIP from GitHub, the installer alone cannot start the dashboard.
You must **build once** on the PC (or copy pre-built `.exe` files into `installer\`).

1. Install **Rust**: https://rustup.rs (default options; open a **new** PowerShell after install)
2. In `installer\`, right-click **`Build-Windows.ps1`** → **Run with PowerShell**  
   (or double-click **`Install-Desktop.bat`** — it will build automatically if binaries are missing)
3. Wait for the build to finish (first time can take several minutes)

---

## Same Wi-Fi / Ethernet (easiest)

You do **not** need to configure IP addresses.

1. Desktop: right-click **`Install-Desktop.bat`** → Run as administrator → note the **pair code**  
2. Laptop: right-click **`Install-Laptop.bat`** → Run as administrator → paste code (or Enter)  
3. Plug ENET → ignition ON → open ISTA/E-Sys when the dashboard is green  

If the browser shows **connection refused** on `http://127.0.0.1:47901/`, the gateway `.exe` was never installed/started — build first (steps above), then re-run the desktop installer.

Details: [INSTALL.md](INSTALL.md)

---

## Different networks (not the same Wi-Fi)

Use a **relay** (both PCs dial out — no home port-forwarding) or **WireGuard**.

Full guide: **[REMOTE.md](REMOTE.md)**

### Relay (recommended remote default)

```bash
# On any VPS / always-on host with a public IP:
enet-relay --listen 0.0.0.0:47910

# Desktop
enet-setup gateway --remote-relay YOUR_VPS:47910 --yes
enet-gateway

# Laptop (same pair code from desktop dashboard)
enet-setup agent --remote-relay YOUR_VPS:47910 --pair-code BMW-XXXX --yes
enet-agent
```

### WireGuard (best quality for flashing)

```bash
enet-setup wireguard --desktop-endpoint YOUR_PUBLIC_IP:51820
# import the two .conf files into WireGuard on each PC, then:
enet-setup gateway --wireguard --yes
enet-setup agent --wireguard --yes
```

Or install Tailscale on both PCs and set the agent `peer_addr` to the desktop's Tailscale IP.

**Flash only when the dashboard says SAFE.** Prefer same-LAN or WireGuard for ECU programming.

---

## Using the GUI after install

Full walkthrough: **[HOW_TO_USE.md](HOW_TO_USE.md)**

1. Desktop: open **http://127.0.0.1:47901/** or the **BMW ENET Gateway** app (`enet-gui`).
2. Copy the **pair code**.
3. Laptop: run the agent with that code.
4. Plug ENET + ignition ON → wait for green lights → open ISTA/E-Sys on the desktop.
