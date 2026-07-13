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

$releaseDir = Join-Path $repoRoot "target\release"
$bins = @("enet-setup.exe", "enet-gateway.exe", "enet-agent.exe", "enet-relay.exe")
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

Write-Host ""
Write-Host "Build complete." -ForegroundColor Green
Write-Host "Next:"
Write-Host "  Desktop PC: right-click Install-Desktop.bat -> Run as administrator"
Write-Host "  Laptop PC:  right-click Install-Laptop.bat  -> Run as administrator"
Write-Host ""

if ($InstallDesktop) {
  & (Join-Path $here "Setup-Gateway.ps1")
}
if ($InstallLaptop) {
  & (Join-Path $here "Setup-Agent.ps1")
}
