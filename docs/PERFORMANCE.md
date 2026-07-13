# Performance Report

## Methodology

Benchmarks use `enet-sim lab` on loopback (same host), measuring forwarded Ethernet frames through the UDP tunnel with keepalive probes enabled.

Environment: Linux CI container, Rust release/dev as noted.

## Lab results (release build, localhost)

Command:

```bash
cargo run -p enet-sim --release -- lab --seconds 3 --flaps --burst 200
```

| Metric | Observed |
|--------|----------|
| Frames delivered (tools side) | 302 (burst + coding + wake) |
| Gateway RX / TX tunnel packets | ~333 / ~20 (keepalives, not storm) |
| Tunnel RTT p99 (loopback) | 1.0 ms |
| Loss rate | 0.0 |
| Errors | 0 |

Integration unit test `tunnel_forwards_ethernet_frame` asserts end-to-end frame identity.

## LAN expectations (Gigabit Ethernet)

| Metric | Target |
|--------|--------|
| Added latency | &lt; 1–3 ms one-way |
| Loss | &lt; 0.1% for flash-safe |
| Throughput | Far above HSFZ/DoIP needs (&lt; 10 Mbps typical) |

## Wi-Fi expectations

| Metric | Guidance |
|--------|----------|
| RTT p99 | Often 5–30 ms; may fail flash-safe gate |
| Loss | Sensitive to interference |
| Flashing | Prefer wired; GUI will warn |

## Flash-safety defaults

- RTT p99 &lt; 20 ms
- Loss &lt; 0.1%
- CPU &lt; 80%
- Vehicle link up + awake
- Minimum traffic samples before certification

## Optimization notes

- UDP with large socket buffers
- Zero protocol parsing of UDS (pure forward)
- Release profile: LTO + `codegen-units=1`
- ChaCha20-Poly1305 optional (small CPU cost on LAN)
