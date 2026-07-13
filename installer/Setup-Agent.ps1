#Requires -RunAsAdministrator
<#
.SYNOPSIS
  One-click laptop installer for BMW ENET Agent.
.DESCRIPTION
  Copies binaries, optionally asks for pair code, writes config with auto-discover,
  installs auto-start service. No desktop IP address required.

  NOTE: Keep this file ASCII-only. Windows PowerShell 5.1 without a UTF-8 BOM
  mis-parses Unicode punctuation and breaks Write-Host strings.
#>
param(
  [string]$InstallDir = "$env:ProgramFiles\BMW-ENET-Agent",
  [string]$PairCode = "",
  [string]$Password = "",
  [string]$Peer = "",
  [switch]$SkipService
)

$ErrorActionPreference = "Stop"
Write-Host ""
Write-Host "=== BMW ENET Agent - Laptop setup ===" -ForegroundColor Cyan
Write-Host "This PC stays near the car. The ENET cable plugs in here."
Write-Host "The desktop is found automatically on your Wi-Fi/Ethernet."
Write-Host ""

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $here
$releaseDir = Join-Path $repoRoot "target\release"

function Find-Binary([string]$Name) {
  $candidates = @(
    (Join-Path $here $Name),
    (Join-Path $releaseDir $Name)
  )
  foreach ($c in $candidates) {
    if (Test-Path $c) { return $c }
  }
  return $null
}

$required = @("enet-agent.exe", "enet-setup.exe")
$missing = @()
$sources = @{}

foreach ($bin in $required) {
  $src = Find-Binary $bin
  if ($src) {
    $sources[$bin] = $src
  } else {
    $missing += $bin
  }
}

if ($missing.Count -gt 0) {
  Write-Host "ERROR: Required Windows binaries were not found." -ForegroundColor Red
  Write-Host ""
  Write-Host "Missing:" -ForegroundColor Yellow
  foreach ($m in $missing) { Write-Host "  - $m" }
  Write-Host ""
  Write-Host "You likely downloaded source code only (no .exe files)."
  Write-Host "Build them first, then re-run this installer:"
  Write-Host ""
  Write-Host "  1. Install Rust from https://rustup.rs" -ForegroundColor Cyan
  Write-Host "  2. In this folder, right-click Build-Windows.ps1 -> Run with PowerShell" -ForegroundColor Cyan
  Write-Host "  3. Run Install-Laptop.bat again" -ForegroundColor Cyan
  Write-Host ""
  Write-Host "Or from the repo root in PowerShell:"
  Write-Host "  .\installer\Build-Windows.ps1"
  exit 1
}

if (-not $PairCode) {
  $PairCode = Read-Host "Pair code from desktop dashboard (Enter to auto-find any gateway)"
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\config" | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\logs" | Out-Null

foreach ($bin in $required) {
  Copy-Item -Force $sources[$bin] (Join-Path $InstallDir $bin)
  Write-Host "  Installed $bin  (from $($sources[$bin]))"
}

$npcap = Get-ItemProperty HKLM:\Software\Npcap -ErrorAction SilentlyContinue
if (-not $npcap) {
  Write-Host ""
  Write-Host "Npcap was not detected." -ForegroundColor Yellow
  Write-Host "For real car traffic, install Npcap from https://npcap.com/ then reboot."
  Write-Host "(You can still finish setup now.)"
}

$setup = Join-Path $InstallDir "enet-setup.exe"
$config = Join-Path $InstallDir "config\agent.toml"
$setupArgs = @("agent", "--config", $config, "--yes")
if ($PairCode) { $setupArgs += @("--pair-code", $PairCode) }
if ($Password) { $setupArgs += @("--password", $Password) }
if ($Peer) { $setupArgs += @("--peer", $Peer) }

if (Test-Path $setup) {
  & $setup @setupArgs
} else {
  Copy-Item -Force (Join-Path $here "..\config\agent.toml") $config -ErrorAction SilentlyContinue
}

if (-not $SkipService -and (Test-Path (Join-Path $InstallDir "enet-agent.exe"))) {
  Write-Host "Installing auto-start service..."
  sc.exe stop BmwEnetAgent 2>$null | Out-Null
  sc.exe delete BmwEnetAgent 2>$null | Out-Null
  $binPath = "`"$InstallDir\enet-agent.exe`" --config `"$config`""
  if ($PairCode) { $binPath += " --pair-code $PairCode" }
  sc.exe create BmwEnetAgent binPath= $binPath start= auto | Out-Null
  sc.exe description BmwEnetAgent "BMW ENET laptop agent (auto-finds desktop)" | Out-Null
  sc.exe start BmwEnetAgent | Out-Null
  Start-Sleep -Seconds 2
  $svc = Get-Service BmwEnetAgent -ErrorAction SilentlyContinue
  if ($svc -and $svc.Status -eq "Running") {
    Write-Host "  Service BmwEnetAgent is running." -ForegroundColor Green
  } else {
    Write-Host "  WARNING: service did not start. Try:" -ForegroundColor Yellow
    Write-Host "    & `"$InstallDir\enet-agent.exe`" --config `"$config`""
  }
}

Write-Host ""
Write-Host "Done." -ForegroundColor Green
Write-Host '1. Plug ENET into the OBD port and this laptop'
Write-Host '2. Turn ignition ON / wake the car'
Write-Host '3. On the desktop dashboard, wait for Laptop + Vehicle lights to turn green'
Write-Host '4. Open ISTA / E-Sys on the desktop'
Write-Host ""
