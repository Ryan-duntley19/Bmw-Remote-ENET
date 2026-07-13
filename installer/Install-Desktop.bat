@echo off
:: Build Windows binaries, then run the desktop installer if build succeeds.
setlocal
cd /d "%~dp0"

if exist "enet-gateway.exe" if exist "enet-setup.exe" goto INSTALL

echo.
echo No enet-gateway.exe found next to this installer.
echo Building from source first (requires Rust from https://rustup.rs )...
echo.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Build-Windows.ps1"
if errorlevel 1 (
  echo.
  echo Build failed. Install Rust, then run Build-Windows.ps1, then retry.
  pause
  exit /b 1
)

:INSTALL
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Setup-Gateway.ps1"
pause
