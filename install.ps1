# 自然资源部庆阳智算中心命令行 (qyzs) PowerShell 一键安装/更新脚本 (Windows)
# 支持架构: x86_64 (amd64)
#
# 可覆盖的环境变量 (便于国内镜像源 / 本地离线验证):
#   QYZS_REPO            仓库标识 (默认 qyzs-ai/qyzs-cli)
#   QYZS_GITHUB_URL      Release 资产下载前缀 (默认 https://github.com/<repo>)
#   QYZS_RELEASE_API_URL 最新版本查询接口 (默认 GitHub Release API)
#   QYZS_FALLBACK_TAG    版本查询失败时的兜底 tag
#   QYZS_VERSION         固定安装指定版本 (如 v0.2.0), 优先于上述查询
#
# 发布时的版本钉定 (业界惯例, 参考 astral-sh/uv):
#   仓库里的本脚本 = 「总是装最新版」;
#   Release 资产里的本脚本 = 「装该 Release 对应的版本」(CI 发布时把下面
#   PinnedVersion 替换为具体 tag)。

$ErrorActionPreference = "Stop"

$Repo = if ($env:QYZS_REPO) { $env:QYZS_REPO } else { "qyzs-ai/qyzs-cli" }
$GithubUrl = if ($env:QYZS_GITHUB_URL) { $env:QYZS_GITHUB_URL } else { "https://github.com/$Repo" }
$ReleaseApiUrl = if ($env:QYZS_RELEASE_API_URL) { $env:QYZS_RELEASE_API_URL } else { "https://api.github.com/repos/$Repo/releases/latest" }
$FallbackTag = if ($env:QYZS_FALLBACK_TAG) { $env:QYZS_FALLBACK_TAG } else { "v0.2.0" }

# >>> RELEASE_PINNED_VERSION (CI 在发布时替换为具体 tag; 仓库版本留空 = 装最新版)
$PinnedVersion = ""
# <<< RELEASE_PINNED_VERSION

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

# 3. 解析要安装的版本
# 优先级: QYZS_VERSION > PINNED_VERSION (发布时钉定) > releases/latest 查询
# 发布时的版本钉定 (业界惯例, 参考 astral-sh/uv): 仓库里的本脚本 = 装最新版;
# Release 资产里的本脚本 = 装该 Release 对应的版本。
$TargetTag = if ($env:QYZS_VERSION) { $env:QYZS_VERSION } else { $PinnedVersion }
if ($TargetTag) {
    Write-Host "[INFO] 安装指定版本: $TargetTag" -ForegroundColor Cyan
} else {
    $LatestTag = $FallbackTag
    try {
        $ReleaseInfo = Invoke-RestMethod -Uri $ReleaseApiUrl -Headers @{"User-Agent"="qyzs-installer"}
        if ($ReleaseInfo.tag_name) {
            $LatestTag = $ReleaseInfo.tag_name
        }
    } catch {
        Write-Host "[WARN] 无法直接获取 GitHub 最新 Release，使用基准版本: $LatestTag" -ForegroundColor Yellow
    }
    $TargetTag = $LatestTag
    Write-Host "[INFO] 准备安装版本: $TargetTag" -ForegroundColor Cyan
}

# 4. 下载并解压安装
$AssetName = "qyzs-$Target.zip"
$DownloadUrl = "$GithubUrl/releases/download/$TargetTag/$AssetName"
$ZipPath = Join-Path $env:TEMP "qyzs_install.zip"
$ExtractPath = Join-Path $env:TEMP "qyzs_extracted_$PID"

Write-Host "[INFO] 正在下载: $DownloadUrl" -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipPath -UseBasicParsing

    # 完整性校验: 与 Release 资产的 SHA256SUMS 对照 (对不上即中止)
    $SumsUrl = "$GithubUrl/releases/download/$TargetTag/SHA256SUMS"
    try {
        $SumsRaw = (Invoke-WebRequest -Uri $SumsUrl -UseBasicParsing).Content
        # PowerShell 7 的 Invoke-WebRequest 返回 .Content 是 Byte[] 而非字符串,
        # 不转成 UTF-8 文本, 后面的正则匹配会静默失败 (退化成"跳过校验")
        $Sums = if ($SumsRaw -is [byte[]]) {
            [System.Text.Encoding]::UTF8.GetString($SumsRaw)
        } else {
            [string]$SumsRaw
        }
        $Expected = ($Sums -split "`n" | Where-Object { $_ -match "\s+$([regex]::Escape($AssetName))\s*$" } | Select-Object -First 1)
        if ($Expected) {
            $ExpectedHash = ($Expected -split "\s+")[0]
            $ActualHash = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash.ToLower()
            if ($ActualHash -ne $ExpectedHash) {
                Write-Host "[ERR] SHA256 校验不通过" -ForegroundColor Red
                Write-Host "[ERR]   期望: $ExpectedHash"
                Write-Host "[ERR]   实际: $ActualHash"
                Write-Host "[ERR] 下载内容可能已被篡改, 已中止安装" -ForegroundColor Red
                exit 1
            }
            Write-Host "[OK] SHA256 校验通过: $($ActualHash.Substring(0,16))..." -ForegroundColor Green
        } else {
            Write-Host "[WARN] 未取得 $AssetName 的 SHA256 期望值, 跳过完整性校验" -ForegroundColor Yellow
        }
    } catch {
        Write-Host "[WARN] 无法获取 SHA256SUMS ($SumsUrl), 跳过完整性校验" -ForegroundColor Yellow
    }

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
