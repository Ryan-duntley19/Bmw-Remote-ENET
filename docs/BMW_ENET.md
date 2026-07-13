# BMW ENET Communication Overview

## Addressing

| Item | Value |
|------|-------|
| Vehicle / tester subnet | `169.254.0.0/16` (link-local / APIPA) |
| Typical tester IP | `169.254.1.1` |
| Mask | `255.255.0.0` |
| Vehicle gateway | MAC-derived `169.254.x.x` |

## Ports & protocols

| Name | Transport | Port | Notes |
|------|-----------|------|-------|
| HSFZ | TCP | 6801 | Primary F-Series diagnostic channel |
| HSFZ discovery | UDP | 6811 | Broadcast to `169.254.255.255` |
| DoIP | TCP/UDP | 13400 | ISO 13400; routing activation on TCP connect |
| UDS | payload | — | ISO 14229 services inside HSFZ/DoIP |
| ISO-TP | — | — | Classic CAN segmentation; not used as the ENET framing |

## Session sketch

1. ENET cable pulls OBD pin 8 (activation) → Ethernet PHY up.
2. Tester obtains/sets link-local address.
3. Discovery broadcast (HSFZ and/or DoIP vehicle ID).
4. TCP connect to gateway.
5. DoIP: send Routing Activation immediately.
6. UDS Default → Extended Diagnostic → Programming (flash).
7. Tester Present keep-alives during long jobs.

## Why L2 tunneling is mandatory

Port-forwarding only TCP 6801/13400 breaks:

- ARP resolution of the vehicle gateway
- UDP 6811 discovery
- DoIP UDP vehicle announcements
- Some tool auto-detect logic

The remote gateway therefore forwards **raw Ethernet frames**.

## References

- ISO 13400 (DoIP)
- ISO 14229 (UDS)
- EDIABAS / EdiabasLib ENET configuration notes (`EnetRemoteHost`, ports 6801/6811/13400)
