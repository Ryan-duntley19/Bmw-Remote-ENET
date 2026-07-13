# Quick Start (5 minutes)

You do **not** need to configure IP addresses.

## What you need

- Desktop PC (runs ISTA / E-Sys) on your home Wi‑Fi or Ethernet  
- Laptop near the car, same network  
- BMW ENET cable (OBD → RJ45)

## Desktop (once)

1. Copy the installer folder to the desktop.
2. Double-click **`Install-Desktop.bat`** (allow Admin).
3. Browser opens → note the **Pair code** (e.g. `BMW-7K2Q`).

Or from a build tree:

```bash
enet-setup gateway --yes
enet-gateway
# open http://127.0.0.1:47901/
```

## Laptop (once)

1. Copy the installer folder to the laptop.
2. Double-click **`Install-Laptop.bat`** (allow Admin).
3. Paste the pair code when asked (or press Enter to auto-find).

Or:

```bash
enet-setup agent          # optional pair code prompt
enet-agent                # finds the desktop automatically
```

## Every time you use it

1. Desktop gateway running (service auto-starts).  
2. Laptop agent running.  
3. Plug ENET into car + laptop, ignition ON.  
4. Desktop dashboard shows **Laptop connected** + **Vehicle awake**.  
5. Open ISTA / E-Sys on the desktop.  
6. Flash ECUs only when Flash safety says OK.

## Troubleshooting (plain language)

| Problem | Fix |
|---------|-----|
| Laptop never finds desktop | Same Wi‑Fi? Gateway running? Try pair code. Disable guest Wi‑Fi isolation. |
| Vehicle never awake | ENET cable seated? Ignition ON? Wait 10–20s after plug-in. |
| ISTA cannot see car | Dashboard must show Connected first; use adapter `BMW-ENET` / `169.254.1.1`. |
| Want to start over | Run `uninstall.bat`, then install again. |

## Uninstall

Double-click `uninstall.bat` on each PC.
