<#
.SYNOPSIS
  Build Windows .exe binaries and copy them into installer\ for Install-*.bat.

.DESCRIPTION
  Requires Rust (https://rustup.rs). First build can take several minutes.
  Keep this file ASCII-only for Windows PowerShell 5.1 compatibility.
#>
param(
  [switch]$SkipGui,
  [switch]$InstallDesktop,
  [switch]$InstallLaptop
)

$ErrorActionPreference = "Stop"

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $here
Set-Location $repoRoot

Write-Host ""
Write-Host "=== Build BMW ENET Gateway (Windows) ===" -ForegroundColor Cyan
Write-Host "Repo: $repoRoot"
Write-Host ""

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  Write-Host "ERROR: cargo not found." -ForegroundColor Red
  Write-Host "Install Rust from https://rustup.rs then open a NEW PowerShell window and retry."
  exit 1
}

$packages = @(
  "-p", "enet-setup",
  "-p", "enet-gateway",
  "-p", "enet-agent",
  "-p", "enet-relay"
)
if (-not $SkipGui) {
  $packages += @("-p", "enet-gui")
}

Write-Host "Building release binaries (this may take a few minutes)..."
& cargo build --release @packages
if ($LASTEXITCODE -ne 0) {
  Write-Host "Build failed." -ForegroundColor Red
  exit $LASTEXITCODE
}

Write-Host "Building setup wizard (BMW-ENET-Setup.exe)..."
& cargo build --release -p enet-installer
if ($LASTEXITCODE -ne 0) {
  Write-Host "Installer build failed." -ForegroundColor Red
  exit $LASTEXITCODE
}

$releaseDir = Join-Path $repoRoot "target\release"
$bins = @("enet-setup.exe", "enet-gateway.exe", "enet-agent.exe", "enet-relay.exe", "BMW-ENET-Setup.exe")
if (-not $SkipGui) { $bins += "enet-gui.exe" }

Write-Host ""
Write-Host "Copying binaries into installer\ ..."
foreach ($bin in $bins) {
  $src = Join-Path $releaseDir $bin
  if (Test-Path $src) {
    Copy-Item -Force $src (Join-Path $here $bin)
    Write-Host "  OK  $bin"
  } else {
    Write-Host "  MISSING  $bin" -ForegroundColor Yellow
  }
}

# Offline packages the wizard can use without GitHub
$hostDir = Join-Path $here "_host_pkg"
$clientDir = Join-Path $here "_client_pkg"
New-Item -ItemType Directory -Force -Path $hostDir | Out-Null
New-Item -ItemType Directory -Force -Path $clientDir | Out-Null
Copy-Item -Force (Join-Path $releaseDir "enet-gateway.exe") $hostDir -ErrorAction SilentlyContinue
Copy-Item -Force (Join-Path $releaseDir "enet-setup.exe") $hostDir -ErrorAction SilentlyContinue
Copy-Item -Force (Join-Path $releaseDir "enet-gui.exe") $hostDir -ErrorAction SilentlyContinue
Copy-Item -Force (Join-Path $releaseDir "enet-agent.exe") $clientDir -ErrorAction SilentlyContinue
Copy-Item -Force (Join-Path $releaseDir "enet-setup.exe") $clientDir -ErrorAction SilentlyContinue
if (Get-Command Compress-Archive -ErrorAction SilentlyContinue) {
  Compress-Archive -Path "$hostDir\*" -DestinationPath (Join-Path $here "BMW-ENET-Host-windows-x64.zip") -Force
  Compress-Archive -Path "$clientDir\*" -DestinationPath (Join-Path $here "BMW-ENET-Client-windows-x64.zip") -Force
  Write-Host "  OK  BMW-ENET-Host-windows-x64.zip / BMW-ENET-Client-windows-x64.zip"
}
Remove-Item -Recurse -Force $hostDir, $clientDir -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "Build complete." -ForegroundColor Green
Write-Host "End users: double-click BMW-ENET-Setup.exe and choose Host or Client."
Write-Host "Legacy: Install-Desktop.bat / Install-Laptop.bat still work after this build."
Write-Host ""

if ($InstallDesktop) {
  & (Join-Path $here "Setup-Gateway.ps1")
}
if ($InstallLaptop) {
  & (Join-Path $here "Setup-Agent.ps1")
}
