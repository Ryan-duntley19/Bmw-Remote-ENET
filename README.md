# BMW ENET Remote Gateway

> **DO NOT DOWNLOAD — STILL IN TESTING.**  
> Using this against a real vehicle can damage ECUs or brick the car. Do not flash or run diagnostics on a car you care about until testing is finished.

Run BMW tools (**ISTA / E-Sys / etc.**) on your desktop while the **ENET cable** stays on a laptop at the car.

```
Car ──ENET──► Laptop (Client) ══ Wi‑Fi / VPN ══► Desktop (Host) ──► ISTA / E-Sys
```

---

## Install (Windows)

You need **one file**. No Rust. No `.bat` scripts.

1. Log into GitHub → open **[Releases](https://github.com/Ryan-duntley19/Bmw-Remote-ENET/releases)**  
   *(while private: you must be signed in; when public: anyone can download)*
2. Download **`BMW-ENET-Setup.exe`** (use **v0.1.26 or newer**)
3. Double-click it → allow Administrator
4. Choose a role and click **Install**:

| This PC | Choose |
|---------|--------|
| Desktop with ISTA / E-Sys | **Host (Desktop)** |
| Laptop with the ENET cable | **Client (Laptop)** |

5. **Host:** browser opens to http://127.0.0.1:47901/ — note the **pair code**  
   **Client:** paste that pair code (recommended). Desktop IP is **auto-detected** (DHCP / changing IPs OK).

Files install under `C:\BMW-ENET\Host` or `C:\BMW-ENET\Client`.

Do the same Setup.exe on both PCs (Host on one, Client on the other).

Same home router required (not Guest / AP-isolation Wi‑Fi). If Desktop stays Waiting, open http://127.0.0.1:47903/ → **Auto-find desktop**.

### If Setup says `VCRUNTIME140.dll` was not found

Install the **Microsoft Visual C++ Redistributable (x64)** once:

https://aka.ms/vs/17/release/vc_redist.x64.exe

Then run `BMW-ENET-Setup.exe` again.  
(Newer releases from **v0.1.5+** bundle this runtime and should not need the redistributable.)

### Auto-update

From **v0.1.20**, Host and Client **update themselves** from GitHub Releases:

- On every start they check for a newer release and install it before connecting.
- While running they check every 6 hours and install when nothing is connected
  (an update never interrupts a diagnostics/flash session).
- Both dashboards show an **Update now** button when a new version is waiting.

You only need `BMW-ENET-Setup.exe` once per PC. Note: automatic checks need the
GitHub repo to be **public** (or set `update_token` in the config file while private).
Set `auto_update = false` in `config/*.toml` to opt out.

### Uninstall

Download the source or copy `installer/uninstall.bat`, then run it as Administrator.

---

## Use it

1. Host dashboard shows green **Gateway running**
2. Client is running on the laptop
3. Plug ENET into the car → ignition ON
4. Wait for **Laptop** + **Vehicle** lights to turn green
5. Open ISTA / E-Sys on the desktop (`BMW-ENET` / `169.254.1.1`)
6. Flash ECUs only when **Flash safety** says **SAFE**

Day-to-day details: **[docs/HOW_TO_USE.md](docs/HOW_TO_USE.md)**

---

## Different networks?

Same Wi‑Fi is easiest. For two different networks (not at home together), see **[docs/REMOTE.md](docs/REMOTE.md)** (relay or WireGuard).

---

## Docs

| Doc | When to read it |
|-----|-----------------|
| [HOW_TO_USE](docs/HOW_TO_USE.md) | Dashboard, pairing, flashing, troubleshooting |
| [REMOTE](docs/REMOTE.md) | Desktop and laptop on different networks |
| [DEVELOPERS](docs/DEVELOPERS.md) | Build from source / CI |
| [ARCHITECTURE](docs/ARCHITECTURE.md) | How the L2 tunnel works |
| [WINDOWS_DRIVERS](docs/WINDOWS_DRIVERS.md) | Npcap / Wintun notes |

---

## Safety

**Still in testing — do not use on a car you care about.**  
Never auto-writes vehicle data. Prefer a wired or low-latency link for flashing. Flash only when the UI says **SAFE**.

## License

MIT
