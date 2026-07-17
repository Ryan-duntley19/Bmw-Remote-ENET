# BMW ENET Gateway — Claude Code brief

Private GitHub repo: `https://github.com/Ryan-duntley19/Bmw-Remote-ENET`  
Active branch: `cursor/remote-enet-gateway-0b54`  
PR: https://github.com/Ryan-duntley19/Bmw-Remote-ENET/pull/3  
Latest tag: **v0.1.25** (Vehicle ENET = media carrier only; Npcap auto-download/launch)

## What this project is

Layer-2 Ethernet-over-UDP tunnel so BMW tools (ISTA / E-Sys) run on a **desktop Host** while the ENET cable stays on a **laptop Client** at the car.

```
Car ──ENET──► Laptop (enet-agent) ══ UDP ══► Desktop (enet-gateway) ──► ISTA
```

Ports: tunnel UDP **47900**, Host API **47901**, discovery **47902**, Client status **47903**.

## User topology (important)

- Desktop Host: wired LAN  
- Laptop Client: Wi‑Fi  
- Same home router; IPs are **DHCP / not static**  
- Multi-homed desktop: the Host may reply from a different NIC/IP than the one dialed (v0.1.13+ learns the reply IP)

## Current product status

| Feature | Status |
|---------|--------|
| Host↔Client UDP tunnel + pair-code discovery | Working (v0.1.17 auto-detect / DHCP re-learn) |
| Client status page Connect / Auto-find | Working (v0.1.16+) |
| PowerShell flicker from link polling | Fixed (v0.1.15) |
| Real L2 to ISTA (Npcap + BMW-ENET) | **Implemented in v0.1.18 — needs user install of Npcap on both PCs** |
| Making repo public | User will flip GitHub visibility when ready |

## Why ISTA did not see the car before v0.1.18

“Connected” only meant the control-plane tunnel was up. Host used `SimulatedEthernet`; Client `MonitoredEthernet` did not capture/inject frames. v0.1.18 adds `PcapEthernet` (Npcap) on both sides and installer creates desktop **BMW-ENET** loopback at **169.254.1.1**.

## What the user must do next (ISTA)

1. Wait for GitHub Release **v0.1.18** `BMW-ENET-Setup.exe` (Windows CI builds on tag push).
2. Install **Npcap** on desktop **and** laptop: https://npcap.com — enable **WinPcap API-compatible Mode**.
3. Re-run Setup → Host (desktop) and Client (laptop).
4. In ISTA select interface **BMW-ENET** (not Wi‑Fi/LAN).
5. ENET cable + ignition ON; wait for Vehicle awake.

## Repo layout

- `crates/enet-agent` — laptop Client + status page `:47903`
- `crates/enet-gateway` — desktop Host + dashboard `:47901`
- `crates/enet-tunnel` — L2-over-UDP engine; `pcap_ethernet.rs` (Windows)
- `crates/enet-core` — config, LAN discovery, safety, stats
- `crates/enet-installer` — `BMW-ENET-Setup.exe` wizard
- `docs/HOW_TO_USE.md`, `docs/WINDOWS_DRIVERS.md`, `docs/ARCHITECTURE.md`

## Build

```bash
cargo check -p enet-agent -p enet-gateway -p enet-tunnel -p enet-core
# Windows release CI: .github/workflows/windows-installer.yml (needs Npcap SDK LIB=)
```

## Known constraints

- Same router required (not Guest / AP-isolation Wi‑Fi).
- Only **one** `enet-agent` (single-instance mutex).
- Never use laptop’s own IP as `--peer`.
- Flash-safety red without car ENET/ignition is expected.
- Wintun is L3-only — Host uses Microsoft Loopback + Npcap for L2, not Wintun.

## User goals still open

- Finish ISTA end-to-end validation after Npcap + v0.1.18 install.
- Make GitHub repo **public** when finished (Settings → Change visibility).
- Prefer no manual PowerShell; auto-detect IPs (done in v0.1.17).
