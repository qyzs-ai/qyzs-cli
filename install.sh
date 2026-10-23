#!/usr/bin/env bash
# 自然资源部庆阳智算中心命令行 (qyzs) 跨平台一键安装/更新脚本 (Linux / macOS)
# 支持架构: amd64 (x86_64), arm64 (aarch64), loong64 (loongarch64, Linux专属)
#
# 可覆盖的环境变量 (便于国内镜像源 / 本地离线验证):
#   QYZS_REPO            仓库标识 (默认 qyzs-ai/qyzs-cli)
#   QYZS_GITHUB_URL      Release 资产下载前缀 (默认 https://github.com/<repo>)
#   QYZS_RELEASE_API_URL 最新版本查询接口 (默认 GitHub Release API)
#   QYZS_FALLBACK_TAG    版本查询失败时的兜底 tag

set -euo pipefail

REPO="${QYZS_REPO:-qyzs-ai/qyzs-cli}"
GITHUB_URL="${QYZS_GITHUB_URL:-https://github.com/${REPO}}"
RELEASE_API_URL="${QYZS_RELEASE_API_URL:-https://api.github.com/repos/${REPO}/releases/latest}"
FALLBACK_TAG="${QYZS_FALLBACK_TAG:-v0.2.0}"

echo "=========================================================="
echo " 自然资源部庆阳智算中心 CLI (qyzs) 安装/更新向导"
echo "=========================================================="

# 1. 探测操作系统
OS="$(uname -s)"
case "${OS}" in
    Linux*)     OS_TARGET="unknown-linux-musl";;
    Darwin*)    OS_TARGET="apple-darwin";;
    *)          echo "[ERR] 不支持的操作系统: ${OS}"; exit 1;;
esac

# 2. 探测 CPU 架构
ARCH="$(uname -m)"
case "${ARCH}" in
    x86_64|amd64)       ARCH_TARGET="x86_64";;
    aarch64|arm64)      ARCH_TARGET="aarch64";;
    loongarch64|loong64)
        if [ "${OS}" != "Linux" ]; then
            echo "[ERR] 龙芯架构目前仅支持 Linux 平台"; exit 1
        fi
        ARCH_TARGET="loongarch64"
        # CI 对 loong64 构建的是 gnu 目标（.github/workflows/ci.yml matrix），
        # 其余 Linux 架构为 musl；此处必须与 CI 一致，否则资产名对不上
        OS_TARGET="unknown-linux-gnu";;
    *)                  echo "[ERR] 不支持的硬件架构: ${ARCH}"; exit 1;;
esac

TARGET="${ARCH_TARGET}-${OS_TARGET}"
echo "[INFO] 探测到当前环境: ${OS} (${ARCH}) -> 目标构建标识: ${TARGET}"

# 3. 确定安装目录与“安装即更新”检测
INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "${INSTALL_DIR}"
TARGET_BIN="${INSTALL_DIR}/qyzs"

if command -v qyzs >/dev/null 2>&1; then
    EXISTING_PATH="$(command -v qyzs)"
    echo "[INFO] 检测到本地已存在旧版本 qyzs: ${EXISTING_PATH}"
    echo "[INFO] 执行 '安装即更新' 模式，覆盖更新至最新版..."
    if [ -w "$(dirname "${EXISTING_PATH}")" ]; then
        TARGET_BIN="${EXISTING_PATH}"
    fi
fi

# 4. 获取最新 Release 版本
LATEST_TAG="$(curl -sSL "${RELEASE_API_URL}" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || echo "${FALLBACK_TAG}")"
if [ -z "${LATEST_TAG}" ]; then
    LATEST_TAG="${FALLBACK_TAG}"
fi
echo "[INFO] 准备下载版本: ${LATEST_TAG}"

# 5. 下载并解压
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

DOWNLOAD_URL="${GITHUB_URL}/releases/download/${LATEST_TAG}/qyzs-${TARGET}.tar.gz"
echo "[INFO] 下载地址: ${DOWNLOAD_URL}"

if curl -sSL -f "${DOWNLOAD_URL}" -o "${TMP_DIR}/qyzs.tar.gz"; then
    tar -xzf "${TMP_DIR}/qyzs.tar.gz" -C "${TMP_DIR}"
    chmod +x "${TMP_DIR}/qyzs"
    mv "${TMP_DIR}/qyzs" "${TARGET_BIN}"
else
    # CI 一律以压缩包形式发布资产（upload-rust-binary-action 的 archive: qyzs-<target>），
    # 不存在裸二进制资产。此处直接失败退出——否则下载失败仍会打印"安装成功"
    echo "[ERR] 下载失败: ${DOWNLOAD_URL}"
    echo "[ERR] 请确认该版本 (${LATEST_TAG}) 已发布, 且包含 ${TARGET} 平台资产"
    exit 1
fi

echo "=========================================================="
echo "[OK] qyzs 成功安装/更新至: ${TARGET_BIN}"
echo "=========================================================="

# 6. 验证运行与 PATH 提示
if ! echo "${PATH}" | grep -q "${INSTALL_DIR}"; then
    echo "[WARN] 注意: ${INSTALL_DIR} 尚未包含在当前 PATH 环境变量中。"
    echo "[TIP]  请将以下内容追加至 ~/.bashrc 或 ~/.zshrc:"
    echo "       export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

"${TARGET_BIN}" --version || true
"${TARGET_BIN}" status || true
