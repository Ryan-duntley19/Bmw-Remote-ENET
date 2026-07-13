# Build scripts

## Linux / CI

```bash
cargo test --workspace --exclude enet-gui
cargo build --release -p enet-agent -p enet-gateway -p enet-sim
cargo run -p enet-sim --release -- lab --seconds 5 --flaps
```

## Windows

Easiest path (from the repo / extracted ZIP):

```powershell
# Requires Rust: https://rustup.rs
.\installer\Build-Windows.ps1
# then:
.\installer\Install-Desktop.bat   # on the tools PC
.\installer\Install-Laptop.bat    # on the car-side laptop
```

Manual equivalent:

```powershell
cargo build --release -p enet-setup -p enet-gateway -p enet-agent -p enet-gui -p enet-relay
copy target\release\enet-*.exe installer\
cd installer
.\Install-Desktop.bat
```

Cross-compile from Linux to Windows (optional):

```bash
rustup target add x86_64-pc-windows-gnu
# requires mingw toolchain
cargo build --release --target x86_64-pc-windows-gnu -p enet-agent -p enet-gateway
```
