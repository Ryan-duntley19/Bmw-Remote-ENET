# BMW ENET Remote Gateway

Connect your **desktop** (ISTA / E-Sys) to your BMW while the **ENET cable** stays on a **laptop** near the car.

Works on the **same Wi‑Fi** *or* on **different networks** (relay / WireGuard).

## How to use (GUI)

Step-by-step for the browser dashboard and desktop app:

→ **[docs/HOW_TO_USE.md](docs/HOW_TO_USE.md)**

Short version:

1. Start the gateway on the desktop → open **http://127.0.0.1:47901/** (or `enet-gui`).
2. Note the **pair code**.
3. Start the agent on the laptop (paste the pair code if asked).
4. Plug ENET into the car, ignition ON.
5. When the UI lights are green, open ISTA / E-Sys on the desktop.
6. Flash only when **Flash safety** says SAFE.

## 5-minute setup

**Windows (from a GitHub ZIP):** install Rust → run `installer\Build-Windows.ps1` once → then the Install bat files below.

- Same network → **[docs/QUICKSTART.md](docs/QUICKSTART.md)**  
- Different networks → **[docs/REMOTE.md](docs/REMOTE.md)**  
- Install details → **[docs/INSTALL.md](docs/INSTALL.md)**

| Situation | What to run |
|-----------|-------------|
| Same home Wi‑Fi | `Build-Windows.ps1` once, then `Install-Desktop.bat` + `Install-Laptop.bat` |
| Different networks | `enet-relay` on a VPS + `enet-setup … --remote-relay` |
| Best remote quality | `enet-setup wireguard` then import WireGuard configs |

```bash
# Same LAN
enet-setup gateway --yes && enet-gateway
enet-gui                          # optional native UI
# other PC:
enet-setup agent && enet-agent

# Different networks (relay)
enet-relay --listen 0.0.0.0:47910
enet-setup gateway --remote-relay vps:47910 --yes && enet-gateway
enet-setup agent --remote-relay vps:47910 --pair-code BMW-XXXX --yes && enet-agent
```

## How it works

Transparent **Layer-2** tunnel (required for BMW ARP / HSFZ / DoIP discovery).

```
Vehicle ──ENET──► Laptop agent ══ LAN or Relay/VPN ══► Desktop gateway ──► ISTA / E-Sys
```

## Safety

Never auto-writes vehicle data. Flash only when the UI says SAFE — especially on remote links.

## License

MIT
