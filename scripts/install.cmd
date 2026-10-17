@echo off
setlocal
echo ==========================================================
echo  QYZS CLI Windows Installer (Batch wrapper)
echo ==========================================================
powershell -NoProfile -ExecutionPolicy Bypass -Command "& { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; & '%~dp0install.ps1' }"
endlocal
