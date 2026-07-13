# Windows Drivers & Packet IO

## Laptop (capture / inject)

Production path: **Npcap** (`pcap_open_live` / `sendpacket`) on the ENET adapter only.

Requirements:

- Npcap installed with **WinPcap API compatibility**
- Process elevated / SYSTEM (Setup scheduled task)
- Interface selected via auto-detect (`169.254.0.0/16`, description heuristics) or `enet_interface`

Do **not** capture the LAN/Wi‑Fi adapter used for the UDP tunnel (hairpin / loops).

Implementation: `crates/enet-tunnel/src/pcap_ethernet.rs` → `PcapEthernet`, opened from `enet-agent`.

## Desktop (virtual NIC for ISTA)

ISTA needs a real Ethernet adapter named **`BMW-ENET`** at **`169.254.1.1/16`**.

1. Setup (Host) creates a **Microsoft KM-TEST Loopback** adapter renamed `BMW-ENET` and assigns the tester IP (WeakHostSend/Receive enabled).
2. Host opens that adapter with **Npcap** (`PcapEthernet`) and bridges L2 frames to/from the laptop tunnel.

Npcap is required on the desktop as well (same WinPcap-compatible install).

## Current repository status

| Path | Status |
|------|--------|
| LAN tunnel + discovery | Done |
| Host `BMW-ENET` + Npcap L2 | Done (v0.1.18+) |
| Client ENET Npcap L2 | Done (v0.1.18+) |
| Linux CI without Npcap | Uses `SimulatedEthernet` |

Install Npcap on both Windows PCs before expecting ISTA to see the vehicle.
