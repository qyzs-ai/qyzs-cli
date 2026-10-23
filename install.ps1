# 自然资源部庆阳智算中心命令行 (qyzs) PowerShell 一键安装/更新脚本 (Windows)
# 支持架构: x86_64 (amd64)
#
# 可覆盖的环境变量 (便于国内镜像源 / 本地离线验证):
#   QYZS_REPO            仓库标识 (默认 qyzs-ai/qyzs-cli)
#   QYZS_GITHUB_URL      Release 资产下载前缀 (默认 https://github.com/<repo>)
#   QYZS_RELEASE_API_URL 最新版本查询接口 (默认 GitHub Release API)
#   QYZS_FALLBACK_TAG    版本查询失败时的兜底 tag

$ErrorActionPreference = "Stop"

$Repo = if ($env:QYZS_REPO) { $env:QYZS_REPO } else { "qyzs-ai/qyzs-cli" }
$GithubUrl = if ($env:QYZS_GITHUB_URL) { $env:QYZS_GITHUB_URL } else { "https://github.com/$Repo" }
$ReleaseApiUrl = if ($env:QYZS_RELEASE_API_URL) { $env:QYZS_RELEASE_API_URL } else { "https://api.github.com/repos/$Repo/releases/latest" }
$FallbackTag = if ($env:QYZS_FALLBACK_TAG) { $env:QYZS_FALLBACK_TAG } else { "v0.2.0" }

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host " 自然资源部庆阳智算中心 CLI (qyzs) Windows 安装/更新向导" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# 1. 架构检测
$Arch = [System.Environment]::GetEnvironmentVariable("PROCESSOR_ARCHITECTURE")
if ($Arch -ne "AMD64") {
    Write-Warning "当前仅优先支持 64位 Windows (AMD64/x86_64)，当前架构为: $Arch"
}
# 必须与 CI 构建目标一致（.github/workflows/ci.yml matrix: x86_64-pc-windows-msvc），
# 否则拼出的资产名不存在
$Target = "x86_64-pc-windows-msvc"

# 2. 确定安装路径与“安装即更新”检测
$InstallDir = "$env:LOCALAPPDATA\qyzs\bin"
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}
$TargetExe = Join-Path $InstallDir "qyzs.exe"

$ExistingCmd = Get-Command "qyzs.exe" -ErrorAction SilentlyContinue
if ($ExistingCmd) {
    Write-Host "[INFO] 检测到本地已存在旧版 qyzs: $($ExistingCmd.Source)" -ForegroundColor Yellow
    Write-Host "[INFO] 执行 '安装即更新' 模式，就地升级..." -ForegroundColor Yellow
    $TargetExe = $ExistingCmd.Source
}

# 3. 获取最新 Release 版本
$LatestTag = $FallbackTag
try {
    $ReleaseInfo = Invoke-RestMethod -Uri $ReleaseApiUrl -Headers @{"User-Agent"="qyzs-installer"}
    if ($ReleaseInfo.tag_name) {
        $LatestTag = $ReleaseInfo.tag_name
    }
} catch {
    Write-Host "[WARN] 无法直接获取 GitHub 最新 Release，使用基准版本: $LatestTag" -ForegroundColor Yellow
}
Write-Host "[INFO] 准备安装版本: $LatestTag" -ForegroundColor Cyan

# 4. 下载并解压安装
$DownloadUrl = "$GithubUrl/releases/download/$LatestTag/qyzs-$Target.zip"
$ZipPath = Join-Path $env:TEMP "qyzs_install.zip"
$ExtractPath = Join-Path $env:TEMP "qyzs_extracted_$PID"

Write-Host "[INFO] 正在下载: $DownloadUrl" -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipPath -UseBasicParsing
    Expand-Archive -Path $ZipPath -DestinationPath $ExtractPath -Force
    
    $SourceExe = Get-ChildItem -Path $ExtractPath -Filter "qyzs.exe" -Recurse | Select-Object -First 1
    if (-not $SourceExe) {
        throw "压缩包中未找到 qyzs.exe"
    }

    # 处理 Windows 进程占用重命名替换 (原子更新)
    if (Test-Path $TargetExe) {
        $BakExe = "$TargetExe.old"
        if (Test-Path $BakExe) { Remove-Item $BakExe -Force -ErrorAction SilentlyContinue }
        Move-Item $TargetExe $BakExe -Force -ErrorAction SilentlyContinue
    }

    Copy-Item $SourceExe.FullName -Destination $TargetExe -Force
} finally {
    Remove-Item $ZipPath -Force -ErrorAction SilentlyContinue
    Remove-Item $ExtractPath -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "==========================================================" -ForegroundColor Green
Write-Host "[OK] qyzs 成功安装/更新至: $TargetExe" -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Green

# 5. 添加至用户 PATH
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    Write-Host "[INFO] 正在将 $InstallDir 添加到用户 PATH 环境变量..." -ForegroundColor Cyan
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "[OK] PATH 环境变量配置完成！新打开的终端窗口可直接使用 'qyzs'" -ForegroundColor Green
}

& "$TargetExe" --version
