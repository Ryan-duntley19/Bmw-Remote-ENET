# BMW ENET Remote Gateway

Connect your **desktop** (ISTA / E-Sys) to your BMW while the **ENET cable** stays on a **laptop** near the car.

**No manual IP configuration required** — the laptop finds the desktop on your Wi‑Fi/Ethernet automatically.

## 5-minute setup

See **[docs/QUICKSTART.md](docs/QUICKSTART.md)**.

| PC | What to run |
|----|-------------|
| Desktop | Double-click `installer/Install-Desktop.bat` |
| Laptop | Double-click `installer/Install-Laptop.bat` |

Then open **http://127.0.0.1:47901/** on the desktop — that dashboard is the main UI.

```bash
# From source
enet-setup gateway --yes && enet-gateway
# other PC:
enet-setup agent && enet-agent
```

## How it works

Transparent **Layer-2 Ethernet-over-UDP** tunnel (required for BMW discovery / ARP / HSFZ / DoIP).

```
Vehicle ──ENET──► Laptop agent ══ auto-discover LAN ══► Desktop gateway ──► ISTA / E-Sys
```

Details: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) · BMW notes: [docs/BMW_ENET.md](docs/BMW_ENET.md)

## Components

| Tool | Role |
|------|------|
| `enet-setup` | First-run wizard (`gateway` / `agent` / `find` / `doctor`) |
| `enet-gateway` | Desktop service + browser dashboard |
| `enet-agent` | Laptop tunnel (auto-discovers desktop) |
| `enet-gui` | Optional native GUI |
| `enet-sim` | Lab traffic without a car |

## Safety

The software **never writes** to the vehicle. Flash only when the dashboard says flash safety is OK.

## Build / test

```bash
cargo test --workspace --exclude enet-gui
cargo run -p enet-setup -- gateway --yes
cargo run -p enet-gateway -- --simulate --run-seconds 2
```

## License

MIT
