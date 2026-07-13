# Windows Drivers & Packet IO

## Laptop (capture / inject)

Production path: **Npcap** (`pcap_open_live` / sendpacket) on the ENET adapter only.

Requirements:

- Npcap installed with WinPcap API compatibility
- Process running elevated
- Interface selected via auto-detect (`169.254.0.0/16`, description heuristics) or `enet_interface`

Do **not** capture the LAN adapter used for the UDP tunnel (hairpin / loops).

## Desktop (virtual NIC)

Production path: **Wintun** preferred (modern, signed), TAP-Windows acceptable.

1. Create adapter named `BMW-ENET` (or config `virtual_interface`).
2. Configure IPv4 `169.254.1.1/16`.
3. Gateway reads/writes L2 frames via Wintun ring buffers.

Hyper-V external switches are optional for advanced bridging and are not required.

## Current repository status

CI / Linux builds use `SimulatedEthernet` so the tunnel, protocol, GUI API, and flash-safety logic are fully testable without hardware drivers.

Windows driver bindings should be added behind `#[cfg(windows)]` modules:

- `crates/enet-agent/src/npcap.rs`
- `crates/enet-gateway/src/wintun.rs`

keeping the `EthernetPort` trait as the seam.
