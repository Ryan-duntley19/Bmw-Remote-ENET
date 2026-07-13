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

Same home router is fine. **v0.1.17+ auto-detects the desktop IP** by pair code (DHCP / changing IPs are OK).

1. Start Host on the desktop and Client on the laptop.
2. Match the **pair code** (and password, if any).
3. Client finds the Host automatically. If Desktop stays Waiting, open **http://127.0.0.1:47903/** → **Auto-find desktop**.

Guest / AP-isolation Wi‑Fi will not work. As a fallback only, enter a Desktop LAN IP on the status page.

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

6. Install **Npcap** on **both** PCs (https://npcap.com — enable WinPcap API compatibility). Re-run Setup / restart Host + Client after installing.
7. On the desktop, confirm adapter **`BMW-ENET`** exists at **`169.254.1.1`** (Setup creates it).
8. Open **ISTA+ / E-Sys / BimmerUtility** and select interface **`BMW-ENET`**.

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
| “No desktop on this LAN” (Wi‑Fi + wired) | Status page → **Auto-find desktop** (v0.1.17+) |
| Vehicle never awake | Npcap on laptop? ENET plugged? Ignition ON? |
| ISTA cannot find the car | Npcap on **both** PCs; desktop adapter **BMW-ENET** @ **169.254.1.1**; ISTA selects BMW-ENET; Vehicle awake green |
| Flash safety red | Use Ethernet or WireGuard; avoid hotel Wi‑Fi |

Remote setups: [REMOTE.md](REMOTE.md) · Build from source: [DEVELOPERS.md](DEVELOPERS.md)
