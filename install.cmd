@echo off
setlocal
echo ==========================================================
echo  QYZS CLI Windows Installer (Batch wrapper)
echo ==========================================================

rem 优先用 PowerShell 7 (pwsh); 回退到 Windows 自带的 Windows PowerShell
set "PS=powershell"
where pwsh >nul 2>&1 && set "PS=pwsh"

rem install.ps1 需与本脚本同目录。单独下载 install.cmd 时从这里补取,
rem 否则用户只拿到一个 .cmd 会直接失败。
set "PS1=%~dp0install.ps1"
if exist "%PS1%" goto run

set "PS1=%TEMP%\qyzs_install.ps1"
echo [INFO] 未找到同目录的 install.ps1, 正在从 Release 下载...
"%PS%" -NoProfile -ExecutionPolicy Bypass -Command "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -Uri 'https://github.com/qyzs-ai/qyzs-cli/releases/latest/download/install.ps1' -OutFile '%PS1%'"
if exist "%PS1%" goto run
echo [ERR] 下载 install.ps1 失败, 请手动下载 https://github.com/qyzs-ai/qyzs-cli/releases 后重试
exit /b 1

:run
"%PS%" -NoProfile -ExecutionPolicy Bypass -File "%PS1%"
set "RC=%ERRORLEVEL%"
endlocal & exit /b %RC%
