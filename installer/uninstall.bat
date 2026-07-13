@echo off
echo Uninstalling BMW ENET Gateway / Agent...
sc.exe stop BmwEnetGateway >nul 2>&1
sc.exe delete BmwEnetGateway >nul 2>&1
sc.exe stop BmwEnetAgent >nul 2>&1
sc.exe delete BmwEnetAgent >nul 2>&1
netsh advfirewall firewall delete rule name="BMW ENET Tunnel" >nul 2>&1
netsh advfirewall firewall delete rule name="BMW ENET Discovery" >nul 2>&1
rmdir /S /Q "%ProgramFiles%\BMW-ENET-Gateway" 2>nul
rmdir /S /Q "%ProgramFiles%\BMW-ENET-Agent" 2>nul
del "%USERPROFILE%\Desktop\BMW ENET Gateway.lnk" 2>nul
echo Uninstall complete.
pause
