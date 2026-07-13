# User Manual

## Easiest path

Follow **[QUICKSTART.md](QUICKSTART.md)** — double-click installers, no IP typing.

## What you see

### Browser dashboard (`http://127.0.0.1:47901/`)

- Large **pair code** for the laptop
- Green/grey lights: Gateway / Laptop / Vehicle / Awake
- Flash safety verdict
- Step-by-step checklist

### Native GUI (`enet-gui`)

Same status, plus Setup help, Settings, Export logs, Open in browser.

## Pairing

1. Desktop shows pair code `BMW-XXXX`.
2. Laptop installer asks for it (or press Enter to auto-find).
3. Agent discovers the desktop on UDP 47902 — **no desktop IP needed**.

## Daily use

1. Services auto-start after install.
2. Plug ENET → ignition ON.
3. Wait for Laptop + Vehicle lights.
4. Open ISTA/E-Sys on the desktop.
5. Flash only when safety says OK.

## Commands worth knowing

```bash
enet-setup gateway --yes
enet-setup agent
enet-setup find
enet-setup doctor --role agent
enet-agent --pair-code BMW-XXXX
```

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| API / dashboard unreachable | Start gateway / Install-Desktop.bat |
| Laptop cannot find desktop | Same Wi‑Fi; pair code; private network profile; UDP 47902 allowed |
| Vehicle never awake | Cable, ignition, wait after plug-in |
| ISTA cannot see car | Tunnel Connected first; tester IP `169.254.1.1` on virtual NIC |

## Safety

Never flash when the UI warns. This gateway never modifies vehicle data by itself.
