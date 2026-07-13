# User Manual

## Start here

**How to use the GUI and work on the car day-to-day:**  
→ **[HOW_TO_USE.md](HOW_TO_USE.md)**

**First install (5 minutes):**  
→ **[QUICKSTART.md](QUICKSTART.md)**

**Laptop and desktop on different networks:**  
→ **[REMOTE.md](REMOTE.md)**

---

## What you see in the UI

### Browser dashboard (`http://127.0.0.1:47901/`)

- Large **pair code** for the laptop
- Green/grey lights: Gateway / Laptop / Vehicle / Awake
- Network mode (LAN / Relay / WireGuard)
- Flash safety verdict
- Step-by-step checklist

### Native GUI (`enet-gui`)

Same status, plus:

- **Setup help** — first-run steps
- **Settings** — port / password
- **Start / Stop / Restart**
- **Export logs**
- **Open in browser**

---

## Pairing (summary)

1. Desktop UI shows pair code `BMW-XXXX`.
2. Laptop agent uses that code (or auto-finds on the same LAN).
3. No desktop IP typing needed on the same network.

---

## Daily use (summary)

1. Gateway + agent running.  
2. Plug ENET → ignition ON.  
3. Wait for Laptop + Vehicle lights.  
4. Open ISTA/E-Sys on the desktop.  
5. Flash only when safety says **SAFE**.

---

## Commands

```bash
enet-gateway              # desktop service + dashboard
enet-gui                  # native GUI
enet-agent                # laptop
enet-setup gateway --yes
enet-setup agent
enet-setup find
enet-setup doctor --role gateway
```

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| GUI / dashboard unreachable | Start gateway / `Install-Desktop.bat` |
| Laptop cannot find desktop | Same Wi‑Fi; pair code; or use relay — [REMOTE.md](REMOTE.md) |
| Vehicle never awake | Cable, ignition, wait after plug-in |
| ISTA cannot see car | UI fully green first; tester IP `169.254.1.1` on virtual NIC |

## Safety

This software never modifies vehicle data by itself. Do not flash when the UI warns.
