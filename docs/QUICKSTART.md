# Quick Start (5 minutes)

## Windows — recommended

1. Download **`BMW-ENET-Setup.exe`** from [Releases](https://github.com/Ryan-duntley19/test/releases)
2. On the **desktop**: run Setup → choose **Host (Desktop)** → Install  
   Open **http://127.0.0.1:47901/** and copy the **pair code**
3. On the **laptop**: run the **same** Setup.exe → choose **Client (Laptop)** → paste the pair code → Install
4. Plug ENET → ignition ON → open ISTA/E-Sys when the dashboard is green

Details: **[SETUP_WIZARD.md](SETUP_WIZARD.md)**

You do **not** need Rust or any `.bat` file.

---

## Same Wi-Fi / Ethernet

You do **not** need to configure IP addresses. Host + Client on the same LAN auto-discover via the pair code.

If the browser shows **connection refused** on `http://127.0.0.1:47901/`, the Host was not installed/started — run `BMW-ENET-Setup.exe` as Administrator and choose Host.

More install notes: [INSTALL.md](INSTALL.md)

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
3. Laptop: Client already paired (or re-run Setup and enter the code).
4. Plug ENET + ignition ON → wait for green lights → open ISTA/E-Sys on the desktop.
