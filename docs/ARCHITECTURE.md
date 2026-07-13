# BMW ENET Remote Gateway — Architecture

## Problem Statement

A 2017 BMW M240i (F23 / B58) exposes Ethernet diagnostics (ENET) only on the OBD-II port.
The ENET cable is plugged into a **laptop** near the vehicle. Diagnostic tools (ISTA+, E-Sys,
BimmerUtility, Tool32, INPA) must run on a **desktop** in another room as if the ENET cable
were plugged directly into that desktop.

The gateway must forward **every Ethernet frame** with minimal latency and zero packet loss
under normal LAN conditions, including ARP, UDP broadcasts, and long-lived TCP sessions used
for ECU flashing.

---

## BMW ENET Networking Primer

### Physical / Link Layer

| Item | Detail |
|------|--------|
| Medium | 100BASE-TX Ethernet via OBD pins (TX+/TX−, RX+/RX−) |
| Activation | OBD pin 8 (Ethernet Activation Line) pulled high by ENET cable |
| Gateway ECU | ZGW / ZGM / BDC (F-Series central gateway) |

### IP Addressing

BMW F-Series vehicles typically use **IPv4 link-local** addresses:

- Range: `169.254.0.0/16`
- Subnet mask: `255.255.0.0`
- Tester (PC) commonly uses: `169.254.1.1` / `255.255.0.0`
- Vehicle gateway: MAC-derived APIPA address in `169.254.x.x`
- No DHCP required for direct ENET; APIPA / static link-local is sufficient

Some setups allow DHCP if the gateway is connected to a LAN with a DHCP server, but
standard DIY ENET practice is static/APIPA on `169.254.0.0/16`.

### Protocols

| Protocol | Port(s) | Role |
|----------|---------|------|
| **HSFZ** (High-Speed Fahrzeug Zugang) | TCP **6801** | Primary F-Series diagnostic transport; carries UDS |
| **HSFZ Discovery** | UDP **6811** → `169.254.255.255` | Vehicle/gateway discovery broadcasts |
| **DoIP** (ISO 13400) | TCP/UDP **13400** | Diagnostics over IP; used on newer / dual-stack vehicles |
| **UDS** (ISO 14229) | (payload) | Diagnostic services (DID read, DTC, coding, flashing) |
| **ISO-TP** (ISO 15765-2) | (over CAN historically) | Segmented transport; over ENET, UDS rides HSFZ/DoIP instead |

### Session Flow (simplified)

1. Link up → Ethernet PHY woken by activation line
2. Tester configures `169.254.1.1/16` on the ENET NIC
3. Discovery: UDP broadcast on 6811 (HSFZ) and/or DoIP vehicle identification (UDP 13400)
4. Gateway responds with identity / VIN / logical address
5. Tester opens TCP 6801 (HSFZ) or TCP 13400 (DoIP)
6. DoIP requires **Routing Activation** immediately after TCP connect
7. UDS sessions: Default → Extended → Programming (for flashing)
8. Tester keep-alives / tester-present prevent gateway sleep during long jobs

**Critical implication:** ARP requests, UDP broadcasts, and Ethernet multicast must traverse the
tunnel. A pure Layer-3 TCP proxy that only forwards ports 6801/13400 will break discovery and
many tools.

---

## Architecture Options Evaluated

### 1. Layer-2 Ethernet Bridging (OS bridge / Hyper-V)

| Pros | Cons |
|------|------|
| Perfect transparency | Laptop and desktop must share a Layer-2 domain |
| Native ARP/broadcast | Not possible across rooms without a tunnel |
| Zero custom protocol | Requires physical switch or VPN L2 |

**Verdict:** Ideal locally; insufficient alone for remote rooms.

### 2. Layer-3 Routing / Port Forwarding

| Pros | Cons |
|------|------|
| Simple firewall rules | Breaks ARP and 169.254 broadcasts |
| Easy to understand | Discovery fails; tools often cannot find vehicle |
| | NAT breaks DoIP/HSFZ assumptions |

**Verdict:** Unreliable for BMW tools. Rejected as primary design.

### 3. TAP Interfaces + Custom Forwarder

| Pros | Cons |
|------|------|
| Full L2 frame access | Needs TAP/Wintun driver on desktop |
| Desktop tools see a real NIC | Driver installation complexity |
| Cross-platform (TAP-Windows / tuntap) | |

**Verdict:** Strong foundation for the desktop side.

### 4. WinPcap / Npcap Raw Capture

| Pros | Cons |
|------|------|
| Capture/inject on ENET NIC (laptop) | Admin rights required |
| Mature on Windows | Npcap redistributable licensing |
| Works without TAP on laptop | |

**Verdict:** Preferred laptop-side capture/inject path on Windows.

### 5. Hyper-V Virtual Switch

| Pros | Cons |
|------|------|
| Native Windows L2 switching | Heavy dependency; Pro/Enterprise SKUs |
| Good for VMs | Poor fit for laptop↔desktop tunneling |

**Verdict:** Not primary; optional for advanced setups.

### 6. OpenVPN TAP / SoftEther L2

| Pros | Cons |
|------|------|
| Battle-tested L2 VPN | Higher latency than custom UDP |
| Encryption built-in | Complex config; TLS handshake overhead |
| SoftEther has Ethernet bridging | SoftEther maintenance / attack surface |

**Verdict:** Acceptable fallback / stretch remote-Internet path; not optimal for LAN flashing.

### 7. WireGuard / Tailscale / ZeroTier

| Pros | Cons |
|------|------|
| Excellent security & NAT traversal | Layer-3 by default |
| WireGuard is fast | No native Ethernet broadcast without extra L2 overlay |
| Tailscale/ZeroTier easy mesh | Extra hop / userspace may add jitter |

**Verdict:** Excellent for **optional Internet remote access** wrapped around our L2 tunnel,
not a replacement for L2 ENET forwarding.

### 8. Ethernet-over-IP / VXLAN / Custom UDP Tunnel

| Pros | Cons |
|------|------|
| Full L2 transparency | Must implement reliability / security |
| Lowest latency on LAN (UDP) | Custom protocol to maintain |
| Sequence numbers, stats, QoS possible | |
| Exact control for BMW quirks | |

**Verdict:** **Recommended primary design.**

### 9. TCP Tunneling of Ethernet Frames

| Pros | Cons |
|------|------|
| Ordered, reliable | Head-of-line blocking hurts latency |
| Easier through strict firewalls | TCP-over-TCP anti-pattern under loss |
| | Worse for flashing bursts |

**Verdict:** Optional fallback transport when UDP is blocked.

---

## Recommended Architecture

**Primary:** Custom **Layer-2 Ethernet-over-UDP** tunnel between laptop agent and desktop gateway.

```
┌─────────────┐   ENET cable    ┌──────────────────┐   LAN UDP:47900   ┌─────────────────────┐
│ BMW F23     │◄───────────────►│ Laptop Agent     │◄─────────────────►│ Desktop Gateway     │
│ ZGW/BDC     │  169.254.x.x    │ (Npcap/raw)     │  encrypted frames │ (Windows Service)   │
└─────────────┘                 │ Captures/injects │                   │ Wintun/TAP vNIC     │
                                └──────────────────┘                   │ ISTA / E-Sys / etc. │
                                                                       └─────────────────────┘
```

### Why this wins for BMW F-Series

1. **Discovery works** — UDP 6811 broadcasts and ARP traverse the tunnel unchanged.
2. **DoIP + HSFZ work** — TCP sessions are just Ethernet payloads; no protocol rewriting.
3. **Latency** — UDP on a local Gigabit/Wi-Fi LAN typically adds &lt;1–3 ms RTT.
4. **Flashing** — Ordered delivery is provided by the inner TCP of HSFZ/DoIP; the tunnel
   only needs extremely low loss. We add sequence numbers, loss detection, and optional
   selective retransmission for control frames.
5. **Desktop illusion** — Wintun/TAP presents a NIC that tools treat as a local ENET adapter
   with `169.254.1.1/16`.

### Security Model

- Bind tunnel to LAN interfaces / allowlist CIDRs only
- Optional pre-shared key with AEAD (ChaCha20-Poly1305)
- Optional WireGuard outer tunnel for Internet stretch goal
- Windows Firewall rules installed by the service
- No vehicle data modification — pure forwarder

### Process Roles

| Component | Runs On | Role |
|-----------|---------|------|
| `enet-agent` | Laptop | Detect ENET NIC, capture/inject frames, tunnel client |
| `enet-gateway` | Desktop | Windows service, tunnel server, TAP/Wintun, health checks |
| `enet-gui` | Desktop (and optionally laptop) | Status, settings, flash-safety gate, logs |
| `enet-sim` | Dev/CI | Simulated BMW ENET traffic for tests |

---

## Failure & Recovery

| Event | Behavior |
|-------|----------|
| ENET unplug | Agent detects link down; notifies gateway; desktop TAP stays up with "Vehicle Disconnected" |
| Vehicle sleep | Idle timeout; discovery resumes on wake; auto-reconnect |
| Ignition cycle | TCP sessions drop; tools reconnect; gateway keeps tunnel alive |
| Laptop disconnect | Gateway marks peer lost; exponential backoff reconnect from agent |
| Desktop reboot | Service auto-start; agent reconnects |
| Network interruption | Sequence gap → stats + flash-safety warning |

---

## Flashing Safety Gate

Before the GUI/API allows a "flashing ready" state:

- RTT p99 &lt; threshold (default 20 ms LAN)
- Loss rate &lt; 0.1% over rolling window
- Sustained throughput probe OK
- CPU &lt; soft limit
- Vehicle awake / link up
- Optional voltage check if available via diagnostic read (never auto-write)

If unsafe → **warn and block** the ready flag. Never modify vehicle data automatically.
