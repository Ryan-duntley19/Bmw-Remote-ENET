# BMW ENET Remote Gateway

Connect your **desktop** (ISTA / E-Sys) to your BMW while the **ENET cable** stays on a **laptop** near the car.

Works on the **same Wi-Fi** *or* on **different networks** (relay / WireGuard).

## Install (Windows) — one Setup.exe

**No Rust. No .bat files.** Download the wizard and choose Host or Client:

1. Get **`BMW-ENET-Setup.exe`** from [Releases](https://github.com/Ryan-duntley19/test/releases)  
   (or the GitHub Actions artifact `BMW-ENET-Windows-Installer`)
2. Double-click it → approve Administrator
3. Pick **Host (Desktop)** or **Client (Laptop)** → **Install**
4. Host opens **http://127.0.0.1:47901/** with your pair code  
   Client: enter that pair code (or leave blank on the same Wi-Fi)

Full wizard guide: **[docs/SETUP_WIZARD.md](docs/SETUP_WIZARD.md)**

## How to use (after install)

→ **[docs/HOW_TO_USE.md](docs/HOW_TO_USE.md)**

1. Host dashboard shows the **pair code**.
2. Client is running on the laptop.
3. Plug ENET into the car, ignition ON.
4. When the UI lights are green, open ISTA / E-Sys on the desktop.
5. Flash only when **Flash safety** says SAFE.

## Other docs

- Same network quick path → **[docs/QUICKSTART.md](docs/QUICKSTART.md)**  
- Different networks → **[docs/REMOTE.md](docs/REMOTE.md)**  
- Developer / from-source install → **[docs/INSTALL.md](docs/INSTALL.md)**

| Situation | What to use |
|-----------|-------------|
| Normal Windows install | `BMW-ENET-Setup.exe` (Host or Client) |
| Different networks | `enet-relay` on a VPS + remote mode |
| Best remote quality | WireGuard / Tailscale (see REMOTE.md) |

```bash
# Developers (from source)
enet-setup gateway --yes && enet-gateway
enet-gui
# other PC:
enet-setup agent && enet-agent
```

## How it works

Transparent **Layer-2** tunnel (required for BMW ARP / HSFZ / DoIP discovery).

```
Vehicle ──ENET──► Laptop client ══ LAN or Relay/VPN ══► Desktop host ──► ISTA / E-Sys
```

## Safety

Never auto-writes vehicle data. Flash only when the UI says SAFE — especially on remote links.

## License

MIT
