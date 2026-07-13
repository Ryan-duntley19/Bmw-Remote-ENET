# How to Use BMW ENET Gateway

This guide walks through using the **browser dashboard** and the **desktop GUI** after install.

---

## What runs where

| PC | Program | What it does |
|----|---------|--------------|
| **Desktop** (your room) | `enet-gateway` | Tunnel server + dashboard |
| **Desktop** | `enet-gui` (optional) | Native window for the same status |
| **Desktop** | ISTA / E-Sys / BimmerUtility | Talks to the car through the gateway |
| **Laptop** (near the car) | `enet-agent` | Plugs into ENET; sends traffic to desktop |

You normally only look at the UI on the **desktop**.

---

## Open the UI

### Option A — Browser dashboard (simplest)

1. Make sure the gateway is running (Windows service, or `enet-gateway`).
2. On the desktop, open a browser to:

   **http://127.0.0.1:47901/**

3. The page auto-refreshes every few seconds.

### Option B — Native GUI

1. Double-click the **BMW ENET Gateway** desktop shortcut, **or** run:

   ```bash
   enet-gui
   ```

2. If you see “Gateway not reachable”, start `enet-gateway` first, then click **Setup help**.

3. Click **Open in browser** anytime to jump to the web dashboard.

---

## First-time pairing (do this once)

### Same Wi‑Fi / Ethernet

1. On the **desktop** UI, copy the big **Pair code** (example: `BMW-7K2Q`).
2. On the **laptop**, run the agent installer (or `enet-setup agent`).
3. Paste the pair code when asked (or press Enter to auto-find).
4. Start `enet-agent` and leave it running.

### Different networks

Follow [REMOTE.md](REMOTE.md) (relay or WireGuard), then use the same pair code on the laptop.

---

## Every time you work on the car

1. **Desktop** — confirm gateway is running (green **Gateway running**).
2. **Laptop** — confirm `enet-agent` is running.
3. Plug the **ENET cable** into the car OBD port and the laptop.
4. Turn **ignition ON** (or wake the car).
5. Watch the desktop UI lights:

   | Light | Meaning | You want |
   |-------|---------|----------|
   | Gateway running | Desktop service is up | Green |
   | Laptop connected | Agent reached the desktop | Green |
   | Vehicle connected | ENET link is up | Green |
   | Vehicle awake | Car is answering | Green |

6. When those are green, open **ISTA+ / E-Sys / BimmerUtility** on the desktop.
7. Point the tool at ENET / adapter **`BMW-ENET`** with IP **`169.254.1.1`**.

---

## What the buttons do

### Browser dashboard

| Control | Action |
|---------|--------|
| **Mark setup complete** | Hides first-run nag; config remembers you finished setup |
| **Export logs** | Writes a log dump (path shown in an alert) |

Status and flash safety update automatically.

### Native GUI (`enet-gui`)

| Button | Action |
|--------|--------|
| **Start** | Ask the service to start (if stopped) |
| **Stop** | Stop the tunnel |
| **Restart** | Stop and let the Windows service restart |
| **Settings** | Change tunnel port / optional password |
| **Setup help** | Show the step-by-step checklist + pair code |
| **Open in browser** | Open the web dashboard |
| **Export Logs** | Save logs for troubleshooting |

---

## Flash safety (read this before programming)

The UI shows **Flash safety**:

- **SAFE** — measured latency/loss/CPU look OK. You may proceed, still at your own risk.
- **NOT SAFE** — do **not** flash ECUs. Fix the connection first (prefer wired LAN or WireGuard).

Never leave a flash running on flaky Wi‑Fi or a high-latency relay.

---

## Settings (optional)

In the native GUI → **Settings**:

- **Tunnel port** — default `47900` (must match both PCs if you change it)
- **Password** — optional shared secret; use this for Internet / relay mode

For remote mode, set password on **both** desktop and laptop.

---

## Everyday checklist (print this)

```
[ ] Desktop gateway running
[ ] Laptop agent running
[ ] Same network OR relay/WireGuard connected
[ ] ENET cable plugged into car + laptop
[ ] Ignition ON
[ ] UI: Laptop connected = green
[ ] UI: Vehicle awake = green
[ ] Flash safety = SAFE  (only if flashing)
[ ] Open ISTA / E-Sys on the desktop
```

---

## If something is wrong

| You see | Try this |
|---------|----------|
| Browser: connection refused on `:47901` | Run `BMW-ENET-Setup.exe` as Admin → choose **Host** |
| No Setup.exe / only source ZIP | Download Setup from [Releases](https://github.com/Ryan-duntley19/test/releases) — do not use the source ZIP alone |
| GUI: “Gateway not reachable” | Start Host service or re-run Setup → Host |
| Laptop never connects | Same Wi‑Fi? Pair code correct? Client installed? |
| Vehicle never awake | Reseat ENET, ignition ON, wait 10–20 seconds |
| ISTA cannot find the car | Wait until UI is fully green; use `169.254.1.1` on `BMW-ENET` |
| Flash safety red | Switch to Ethernet, or use WireGuard; avoid hotel Wi‑Fi |

More detail: [USER_MANUAL.md](USER_MANUAL.md) · Install: [INSTALL.md](INSTALL.md) · Remote: [REMOTE.md](REMOTE.md)
