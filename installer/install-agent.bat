@echo off
echo This script is outdated.
echo Please double-click Install-Laptop.bat instead.
pause
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Setup-Agent.ps1"
