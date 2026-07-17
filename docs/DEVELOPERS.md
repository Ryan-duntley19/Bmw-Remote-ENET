# Developer guide

End users should **not** build from source. Download **`BMW-ENET-Setup.exe`** from [Releases](https://github.com/Ryan-duntley19/Bmw-Remote-ENET/releases).

## Build on Windows

1. Install Rust: https://rustup.rs  
2. Open a new PowerShell in the repo root:

```powershell
.\installer\Build-Windows.ps1
```

This builds the role binaries and a self-contained **`BMW-ENET-Setup.exe`** (Host/Client packages embedded).

Windows release builds static-link the MSVC CRT (`+crt-static`) so end users do not need `VCRUNTIME140.dll`.

## Build / test on Linux (CI)

```bash
cargo test --workspace --exclude enet-gui --exclude enet-installer
cargo build --release -p enet-setup -p enet-gateway -p enet-agent -p enet-relay
```

## Run from source (lab)

```bash
# Desktop
cargo run -p enet-setup -- gateway --yes
cargo run -p enet-gateway

# Laptop
cargo run -p enet-setup -- agent --pair-code BMW-XXXX --yes
cargo run -p enet-agent
```

## CI release

`.github/workflows/windows-installer.yml` builds on Windows and publishes on `v*` tags:

- `BMW-ENET-Setup.exe` — self-contained wizard (packages embedded)
- `BMW-ENET-Host-windows-x64.zip` / `BMW-ENET-Client-windows-x64.zip`
- `BMW-ENET-Windows-Installer.zip` — all of the above

## Legacy scripts

Older `.bat` / `.ps1` installers live under `installer/legacy/` and are not needed when using Setup.exe.
