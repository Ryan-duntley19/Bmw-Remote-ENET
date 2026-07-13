@echo off
echo Uninstalling BMW ENET Host / Client...
sc.exe stop BmwEnetGateway >nul 2>&1
sc.exe delete BmwEnetGateway >nul 2>&1
sc.exe stop BmwEnetAgent >nul 2>&1
sc.exe delete BmwEnetAgent >nul 2>&1
powershell -NoProfile -Command "Unregister-ScheduledTask -TaskName 'BMW-ENET-Host' -Confirm:$false -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName 'BMW-ENET-Client' -Confirm:$false -ErrorAction SilentlyContinue" >nul 2>&1
taskkill /F /IM enet-gateway.exe >nul 2>&1
taskkill /F /IM enet-agent.exe >nul 2>&1
taskkill /F /IM enet-gui.exe >nul 2>&1
netsh advfirewall firewall delete rule name="BMW ENET Tunnel" >nul 2>&1
netsh advfirewall firewall delete rule name="BMW ENET Discovery" >nul 2>&1
rmdir /S /Q "%ProgramFiles%\BMW-ENET-Gateway" 2>nul
rmdir /S /Q "%ProgramFiles%\BMW-ENET-Agent" 2>nul
rmdir /S /Q "C:\BMW-ENET" 2>nul
del "%USERPROFILE%\Desktop\BMW ENET Gateway.lnk" 2>nul
echo Uninstall complete.
pause
