@echo off
:: Build Windows binaries if needed, then run the laptop installer.
setlocal
cd /d "%~dp0"

if exist "enet-agent.exe" if exist "enet-setup.exe" goto INSTALL

echo.
echo No enet-agent.exe found next to this installer.
echo Building from source first (requires Rust from https://rustup.rs )...
echo.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Build-Windows.ps1" -SkipGui
if errorlevel 1 (
  echo.
  echo Build failed. Install Rust, then run Build-Windows.ps1, then retry.
  pause
  exit /b 1
)

:INSTALL
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Setup-Agent.ps1"
pause
