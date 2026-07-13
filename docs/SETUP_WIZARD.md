# Windows setup wizard (no Rust required)

Download **one file** and install either the Host or the Client.

## Download

1. Open the repo **Releases** page (or the latest GitHub Actions artifact named `BMW-ENET-Windows-Installer`).
2. Download **`BMW-ENET-Setup.exe`**  
   (or **`BMW-ENET-Windows-Installer.zip`** which also includes the Host/Client packages for offline use).

## Install

1. Double-click **`BMW-ENET-Setup.exe`** (approve UAC / Administrator).
2. Choose:
   - **Host (Desktop)** — PC that runs ISTA / E-Sys
   - **Client (Laptop)** — PC with the ENET cable at the car
3. Click **Install**. The wizard downloads the matching package automatically.
4. Host: browser opens to **http://127.0.0.1:47901/** — copy the pair code.  
   Client: paste that pair code (or leave blank to auto-find on the same Wi-Fi).

You do **not** need to install Rust or run any `.bat` scripts.

## Offline install

If the PC has no internet:

1. Download `BMW-ENET-Windows-Installer.zip` on another machine.
2. Extract so these sit in the **same folder**:
   - `BMW-ENET-Setup.exe`
   - `BMW-ENET-Host-windows-x64.zip`
   - `BMW-ENET-Client-windows-x64.zip`
3. Run `BMW-ENET-Setup.exe` — it uses the local zip for the role you pick.

## Uninstall

Run `installer\uninstall.bat` from the source tree, or remove the Windows services:

```text
sc stop BmwEnetGateway & sc delete BmwEnetGateway
sc stop BmwEnetAgent & sc delete BmwEnetAgent
```

Then delete `C:\Program Files\BMW-ENET-Gateway` or `BMW-ENET-Agent`.

## Developers

Building the wizard yourself still requires Rust — see [INSTALL.md](INSTALL.md) and `installer/Build-Windows.ps1`.
End users should only use **`BMW-ENET-Setup.exe`** from Releases.
