#Requires -RunAsAdministrator
<#
.SYNOPSIS
  One-click desktop installer for BMW ENET Gateway.
.DESCRIPTION
  Copies binaries, writes config, opens firewall, installs auto-start service,
  creates a desktop shortcut, and opens the dashboard in your browser.

  NOTE: Keep this file ASCII-only. Windows PowerShell 5.1 without a UTF-8 BOM
  mis-parses Unicode punctuation and breaks Write-Host strings.
#>
param(
  [string]$InstallDir = "$env:ProgramFiles\BMW-ENET-Gateway",
  [string]$Password = "",
  [switch]$SkipService
)

$ErrorActionPreference = "Stop"
Write-Host ""
Write-Host "=== BMW ENET Gateway - Desktop setup ===" -ForegroundColor Cyan
Write-Host "This PC will run ISTA / E-Sys. Your laptop near the car connects automatically."
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

$required = @("enet-gateway.exe", "enet-setup.exe")
$optional = @("enet-gui.exe")
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
foreach ($bin in $optional) {
  $src = Find-Binary $bin
  if ($src) { $sources[$bin] = $src }
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
  Write-Host "  3. Run Install-Desktop.bat again" -ForegroundColor Cyan
  Write-Host ""
  Write-Host "Or from the repo root in PowerShell:"
  Write-Host "  .\installer\Build-Windows.ps1"
  Write-Host ""
  Write-Host "Dashboard was NOT opened because nothing is installed yet." -ForegroundColor Yellow
  exit 1
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\config" | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\logs" | Out-Null

foreach ($bin in ($required + $optional)) {
  if ($sources.ContainsKey($bin)) {
    Copy-Item -Force $sources[$bin] (Join-Path $InstallDir $bin)
    Write-Host "  Installed $bin  (from $($sources[$bin]))"
  } else {
    Write-Host "  NOTE: $bin not found (optional; browser dashboard still works)" -ForegroundColor Yellow
  }
}

$setup = Join-Path $InstallDir "enet-setup.exe"
$config = Join-Path $InstallDir "config\gateway.toml"
if (Test-Path $setup) {
  & $setup gateway --config $config --password $Password --yes
} elseif (Test-Path (Join-Path $here "..\config\gateway.toml")) {
  Copy-Item -Force (Join-Path $here "..\config\gateway.toml") $config
}

Write-Host ""
Write-Host "Configuring Windows Firewall (LAN only)..."
Get-NetFirewallRule -DisplayName "BMW ENET Tunnel" -ErrorAction SilentlyContinue | Remove-NetFirewallRule
Get-NetFirewallRule -DisplayName "BMW ENET Discovery" -ErrorAction SilentlyContinue | Remove-NetFirewallRule
New-NetFirewallRule -DisplayName "BMW ENET Tunnel" -Direction Inbound -Protocol UDP -LocalPort 47900 -RemoteAddress LocalSubnet -Action Allow -Profile Private | Out-Null
New-NetFirewallRule -DisplayName "BMW ENET Discovery" -Direction Inbound -Protocol UDP -LocalPort 47902 -RemoteAddress LocalSubnet -Action Allow -Profile Private | Out-Null

if (-not $SkipService -and (Test-Path (Join-Path $InstallDir "enet-gateway.exe"))) {
  Write-Host "Installing auto-start service..."
  sc.exe stop BmwEnetGateway 2>$null | Out-Null
  sc.exe delete BmwEnetGateway 2>$null | Out-Null
  $binPath = "`"$InstallDir\enet-gateway.exe`" --config `"$config`""
  sc.exe create BmwEnetGateway binPath= $binPath start= auto | Out-Null
  sc.exe description BmwEnetGateway "BMW ENET desktop gateway (auto-discovers laptop)" | Out-Null
  sc.exe start BmwEnetGateway | Out-Null
  Start-Sleep -Seconds 2
  $svc = Get-Service BmwEnetGateway -ErrorAction SilentlyContinue
  if ($svc -and $svc.Status -eq "Running") {
    Write-Host "  Service BmwEnetGateway is running." -ForegroundColor Green
  } else {
    Write-Host "  WARNING: service did not start. Try:" -ForegroundColor Yellow
    Write-Host "    & `"$InstallDir\enet-gateway.exe`" --config `"$config`""
  }
}

$gui = Join-Path $InstallDir "enet-gui.exe"
if (Test-Path $gui) {
  $desktop = [Environment]::GetFolderPath("Desktop")
  $lnkPath = Join-Path $desktop "BMW ENET Gateway.lnk"
  $w = New-Object -ComObject WScript.Shell
  $s = $w.CreateShortcut($lnkPath)
  $s.TargetPath = $gui
  $s.WorkingDirectory = $InstallDir
  $s.Description = "BMW ENET Gateway dashboard"
  $s.Save()
  Write-Host "Desktop shortcut created."
}

Write-Host ""
Write-Host "Done." -ForegroundColor Green
Write-Host '1. Open http://127.0.0.1:47901/  (pair code is shown there)'
Write-Host '2. On the laptop, right-click Setup-Agent.ps1 -> Run with PowerShell'
Write-Host '3. Plug ENET into the car + laptop, ignition ON, then open ISTA/E-Sys here.'
Write-Host ""
Start-Process 'http://127.0.0.1:47901/'
