# Network Diagrams

## Physical / logical layout

```text
 ┌──────────────┐         ENET cable          ┌─────────────────┐
 │ BMW F23 B58  │◄───────────────────────────►│ Laptop          │
 │ ZGW / BDC    │   100BASE-TX  169.254.x.x   │ enet-agent      │
 └──────────────┘                             │ Npcap on ENET   │
                                              └────────┬────────┘
                                                       │ UDP/47900
                                                       │ (LAN / Wi-Fi)
                                              ┌────────▼────────┐
                                              │ Desktop         │
                                              │ enet-gateway    │
                                              │ Wintun BMW-ENET │
                                              │ 169.254.1.1/16  │
                                              │ ISTA / E-Sys    │
                                              └─────────────────┘
```

## Packet path

```text
ISTA ──► TAP/Wintun ──► Gateway tunnel encode ──UDP──► Agent decode ──► ENET NIC ──► Vehicle
Vehicle ──► ENET NIC ──► Agent encode ──UDP──► Gateway decode ──► TAP/Wintow ──► ISTA
```

## Security zones

```text
[Vehicle link-local]──L2 tunnel──[LAN allowlist]──optional──[WireGuard]──[Internet]
                                      ▲
                               GUI API localhost only
```
