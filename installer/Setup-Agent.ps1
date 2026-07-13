#Requires -RunAsAdministrator
<#
.SYNOPSIS
  One-click laptop installer for BMW ENET Agent.
.DESCRIPTION
  Copies binaries, optionally asks for pair code, writes config with auto-discover,
  installs auto-start service. No desktop IP address required.
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
Write-Host "=== BMW ENET Agent — Laptop setup ===" -ForegroundColor Cyan
Write-Host "This PC stays near the car. The ENET cable plugs in here."
Write-Host "The desktop is found automatically on your Wi-Fi/Ethernet."
Write-Host ""

if (-not $PairCode) {
  $PairCode = Read-Host "Pair code from desktop dashboard (Enter to auto-find any gateway)"
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\config" | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\logs" | Out-Null

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
foreach ($bin in @("enet-agent.exe", "enet-setup.exe")) {
  $src = Join-Path $here $bin
  if (Test-Path $src) {
    Copy-Item -Force $src (Join-Path $InstallDir $bin)
    Write-Host "  Installed $bin"
  } else {
    Write-Host "  WARNING: $bin not found next to this script" -ForegroundColor Yellow
  }
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
}

Write-Host ""
Write-Host "Done." -ForegroundColor Green
Write-Host "1) Plug ENET into the OBD port and this laptop"
Write-Host "2) Turn ignition ON / wake the car"
Write-Host "3) On the desktop dashboard, wait for Laptop + Vehicle lights to turn green"
Write-Host "4) Open ISTA / E-Sys on the desktop"
Write-Host ""
