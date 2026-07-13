#Requires -RunAsAdministrator
<#
.SYNOPSIS
  One-click desktop installer for BMW ENET Gateway.
.DESCRIPTION
  Copies binaries, writes config, opens firewall, installs auto-start service,
  creates a desktop shortcut, and opens the dashboard in your browser.
#>
param(
  [string]$InstallDir = "$env:ProgramFiles\BMW-ENET-Gateway",
  [string]$Password = "",
  [switch]$SkipService
)

$ErrorActionPreference = "Stop"
Write-Host ""
Write-Host "=== BMW ENET Gateway — Desktop setup ===" -ForegroundColor Cyan
Write-Host "This PC will run ISTA / E-Sys. Your laptop near the car connects automatically."
Write-Host ""

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\config" | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\logs" | Out-Null

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
foreach ($bin in @("enet-gateway.exe", "enet-gui.exe", "enet-setup.exe")) {
  $src = Join-Path $here $bin
  if (Test-Path $src) {
    Copy-Item -Force $src (Join-Path $InstallDir $bin)
    Write-Host "  Installed $bin"
  } else {
    Write-Host "  WARNING: $bin not found next to this script (build/copy it first)" -ForegroundColor Yellow
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
Write-Host "1) Open http://127.0.0.1:47901/  (pair code is shown there)"
Write-Host "2) On the laptop, right-click Setup-Agent.ps1 -> Run with PowerShell"
Write-Host "3) Plug ENET into the car + laptop, ignition ON, then open ISTA/E-Sys here."
Write-Host ""
Start-Process "http://127.0.0.1:47901/"
