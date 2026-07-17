<<<<<<< HEAD
# BMW ENET Gateway

Run BMW tools (**ISTA / E-Sys / etc.**) on your desktop while the **ENET cable** stays on a laptop at the car.
=======
# BMW ENET Remote Gateway
DO NOT DOWNLOAD STILL IN TESTING IT WILL BREAK YOUR CAR IF YOU TEST IT OUT
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
>>>>>>> origin/main

```
Car ──ENET──► Laptop (Client) ══ Wi‑Fi / VPN ══► Desktop (Host) ──► ISTA / E-Sys
```

---

## Install (Windows)

You need **one file**. No Rust. No `.bat` scripts.

1. Log into GitHub → open **[Releases](https://github.com/Ryan-duntley19/Bmw-Remote-ENET/releases)**  
   *(while private: you must be signed in; when public: anyone can download)*
2. Download **`BMW-ENET-Setup.exe`** (use **v0.1.24 or newer**)
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

Never auto-writes vehicle data. Prefer a wired or low-latency link for flashing.

## License

MIT
