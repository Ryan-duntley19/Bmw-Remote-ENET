# Future Roadmap

## Near term

- [ ] Windows Npcap capture/inject backend for `enet-agent`
- [ ] Wintun session backend for `enet-gateway`
- [ ] MSI installer via WiX / cargo-wix
- [ ] Signed Windows service binaries
- [ ] Auto-update channel

## Stretch goals

- [ ] WireGuard outer tunnel for remote Internet diagnostics
- [ ] Mobile companion status app
- [ ] Cloud relay (optional, strongly authenticated)
- [ ] Remote coding session recording
- [ ] Multi-vehicle profiles + VIN recognition
- [ ] ECU inventory from discovery
- [ ] Live CAN/DID gauges (read-only)
- [ ] Fault code dashboard (read-only)
- [ ] Automatic ISTA / E-Sys process detection
- [ ] Plugin architecture + REST API (already started via `/api/*`)
- [ ] Web dashboard
- [ ] Packet capture viewer / ECU communication timeline
- [ ] Performance graphs in GUI

## Non-goals

- Automatic ECU writing / coding without user-driven tools
- Bypassing BMW security / token requirements
- Supporting stolen-vehicle or unauthorized remote access scenarios
