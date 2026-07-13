# How to Use BMW ENET Gateway

After installing with **`BMW-ENET-Setup.exe`** (see the [README](../README.md)).

---

## What runs where

| PC | Program | What it does |
|----|---------|--------------|
| **Desktop (Host)** | `enet-gateway` | Tunnel server + dashboard |
| **Desktop** | `enet-gui` (optional) | Native window for the same status |
| **Desktop** | ISTA / E-Sys / BimmerUtility | Talks to the car through the gateway |
| **Laptop (Client)** | `enet-agent` | Plugs into ENET; sends traffic to desktop |

You normally only look at the UI on the **desktop**.

---

## Open the UI

### Browser dashboard (simplest)

1. Make sure Host install finished and `enet-gateway` is running.
2. Open **http://127.0.0.1:47901/**
3. Left side: connection status + performance  
   Right side: **Activity log** for troubleshooting (auto-refreshes)

### Native GUI

Double-click the **BMW ENET Gateway** desktop shortcut, or run `enet-gui`.

---

## First-time pairing

1. On the **Host** UI, copy the **pair code** (example: `BMW-7K2Q`).
2. On the **laptop**, run Setup → **Client** and paste the code (or leave blank on the same Wi‑Fi).
3. Leave `enet-agent` / the Client service running.
4. On the laptop, open **http://127.0.0.1:47903/** (or the **BMW ENET Client Status** shortcut) for Desktop / ENET / Vehicle lights.

### Desktop on Ethernet, laptop on Wi‑Fi

Same home router is fine, but **auto-discovery often fails** across Wi‑Fi ↔ wired. Use the Host dashboard’s **Copy command**, or on the laptop:

```powershell
Stop-Process -Name enet-agent -Force -ErrorAction SilentlyContinue
cd C:\BMW-ENET\Client
# Use the EXACT pair code + desktop Ethernet IPv4 from Host dashboard / ipconfig
.\enet-agent.exe --config config\agent.toml --pair-code BMW-XXXX --peer 192.168.x.x
```

Also match passwords on both PCs (or clear `password` on both). Guest / AP-isolation Wi‑Fi will not work.

### Lower latency (Wi‑Fi laptop)

| Tip | Why |
|-----|-----|
| Stay on **5 GHz**, close to the AP | Cuts RTT / loss vs 2.4 GHz |
| Run **one** `enet-agent` only | Two Clients invent fake packet loss |
| Prefer laptop **Ethernet** to the router when flashing | Flash gate wants p99 &lt; 20 ms |
| Disable Wi‑Fi power saving / “battery saver” | Stops radio sleep spikes |
| Empty password / `require_crypto = false` on LAN | Tiny CPU/wire savings |

Different networks (not the same router): see [REMOTE.md](REMOTE.md).

---

## Every time you work on the car

1. Confirm Host is running (green **Gateway running**).
2. Confirm Client is running on the laptop.
3. Plug the **ENET cable** into the car and the laptop.
4. Turn **ignition ON**.
5. Wait for green lights:

   | Light | Meaning |
   |-------|---------|
   | Gateway running | Desktop service is up |
   | Laptop connected | Client reached the Host |
   | Vehicle connected | ENET link is up |
   | Vehicle awake | Car is answering |

6. Open **ISTA+ / E-Sys / BimmerUtility** on the desktop.
7. Use adapter **`BMW-ENET`** with IP **`169.254.1.1`**.

---

## Flash safety

- **SAFE** — latency/loss/CPU look OK; still your own risk.
- **NOT SAFE** — do **not** flash. Fix the link first (prefer wired LAN or WireGuard).

Never flash over flaky Wi‑Fi or a high-latency relay.

---

## Everyday checklist

```
[ ] Host running
[ ] Client running
[ ] Same network OR relay/WireGuard connected
[ ] ENET plugged into car + laptop
[ ] Ignition ON
[ ] UI: Laptop connected = green
[ ] UI: Vehicle awake = green
[ ] Flash safety = SAFE  (only if flashing)
[ ] Open ISTA / E-Sys on the desktop
```

---

## Troubleshooting

| You see | Try this |
|---------|----------|
| Connection refused on `:47901` | Re-run `BMW-ENET-Setup.exe` → **Host** as Admin |
| Only have a source ZIP | Download Setup from Releases (do not use the source ZIP alone) |
| Laptop never connects | Same router? Exact pair code? Passwords match? |
| “No desktop on this LAN” (Wi‑Fi + wired) | Use `--peer <desktop LAN IP>` from Host dashboard / `ipconfig` |
| Vehicle never awake | Reseat ENET, ignition ON, wait 10–20 seconds |
| Setup: `VCRUNTIME140.dll` not found | Install [VC++ Redistributable x64](https://aka.ms/vs/17/release/vc_redist.x64.exe), or use Setup **v0.1.5+** |
| ISTA cannot find the car | Wait until UI is fully green; use `169.254.1.1` on `BMW-ENET` |
| Flash safety red | Use Ethernet or WireGuard; avoid hotel Wi‑Fi |

Remote setups: [REMOTE.md](REMOTE.md) · Build from source: [DEVELOPERS.md](DEVELOPERS.md)
