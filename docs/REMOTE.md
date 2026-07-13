# Remote / Different-Network Mode

When the **laptop (car)** and **desktop (tools)** are not on the same Wi‑Fi/Ethernet, use one of these modes.

## Recommended options

| Mode | Best when | Port forwarding? | Flashing? |
|------|-----------|------------------|-----------|
| **Relay** (easiest) | Home NAT, hotels, mobile hotspot, different ISPs | No — both sides connect **out** | Only if safety OK (latency higher) |
| **WireGuard** (best quality) | You can install WireGuard on both PCs (or a small VPS) | Only if peer has a public IP / VPS | Prefer this for coding/flash |
| **Same LAN** | Both at home on one network | No | Best |

```text
                    ┌─────────────────┐
 Laptop (ENET) ────►│  enet-relay     │◄──── Desktop (ISTA)
  outbound TCP      │  (VPS / friend  │      outbound TCP
                    │   PC / cloud)   │
                    └─────────────────┘
```

Or:

```text
 Laptop ◄── WireGuard tunnel ──► Desktop
 then normal ENET L2 tunnel over the WG IPs (10.66.0.x)
```

## Option A — Relay (recommended default for remote)

### 1. Run a relay somewhere reachable

On any always-on machine with a public IP (cheap VPS, home server with port forward **only for the relay**):

```bash
enet-relay --listen 0.0.0.0:47910
```

Open **TCP 47910** on that host’s firewall.

### 2. Desktop

```bash
enet-setup gateway --yes --remote-relay wss-or-host:47910
# or edit config:
#   network_mode = "relay"
#   relay_url = "my-vps.example.com:47910"
enet-gateway
```

Dashboard still shows your **pair code**.

### 3. Laptop

```bash
enet-setup agent --yes --remote-relay my-vps.example.com:47910 --pair-code BMW-XXXX
enet-agent
```

Both PCs only make **outbound** connections. No router surgery on either side.

> Use a **password** (`require_crypto = true`) whenever traffic leaves your house.

## Option B — WireGuard (best for flashing)

```bash
enet-setup wireguard --desktop-endpoint vpn.example.com:51820
# writes config/wireguard-desktop.conf and config/wireguard-laptop.conf
```

1. Install [WireGuard](https://www.wireguard.com/install/) on both PCs.  
2. Import the matching conf; activate the tunnel.  
3. Configure ENET gateway/agent with:

```toml
network_mode = "wireguard"
peer_addr = "10.66.0.1"   # desktop WG IP on the laptop agent
auto_discover = false
```

Desktop:

```toml
network_mode = "wireguard"
# listen as usual; agent dials 10.66.0.1
```

If neither home has a public IP, put WireGuard on a **$3–5/mo VPS** and point both peers at it (hub-and-spoke). `enet-setup wireguard --via-vps` prints that layout.

## Option C — Tailscale / ZeroTier (zero config VPN)

1. Install Tailscale (or ZeroTier) on both PCs and join the same tailnet.  
2. Set agent `peer_addr` to the desktop’s Tailscale IP (`100.x`).  
3. `network_mode = "lan"` is fine — Tailscale already bridges the networks.

## Flash safety over remote links

Remote paths add latency and loss risk. The app:

- Raises default RTT budgets slightly in `relay` / `wireguard` modes  
- Still **blocks** the “SAFE to flash” flag unless measured quality is good  
- Prints a stronger warning: prefer WireGuard or wait until you’re on the same LAN for ECU flashing

**Do not flash over hotel Wi‑Fi + relay unless the dashboard says SAFE and you accept the risk.**

## Security checklist

- [ ] Set a shared `password` and `require_crypto = true` for any Internet path  
- [ ] Keep pair codes private (they select the relay room)  
- [ ] Prefer WireGuard or Tailscale for long-term remote use  
- [ ] Run `enet-relay` only on a host you control; firewall to known clients if possible  

## Setup commands cheat sheet

```bash
# Relay on a VPS
enet-relay --listen 0.0.0.0:47910

# Desktop + laptop via relay
enet-setup gateway --remote-relay vps:47910 --yes
enet-setup agent --remote-relay vps:47910 --pair-code BMW-XXXX --yes

# Generate WireGuard configs
enet-setup wireguard --desktop-endpoint YOUR_PUBLIC_IP:51820
```
