#!/usr/bin/env bash
# 自然资源部庆阳智算中心命令行 (qyzs) 跨平台一键安装/更新脚本 (Linux / macOS)
# 支持架构: amd64 (x86_64), arm64 (aarch64), loong64 (loongarch64, Linux专属)
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
#   PINNED_VERSION 替换为具体 tag)。这样从 Release 下载的脚本不会意外装到
#   比它更新的版本, 而 releases/latest/download/install.sh 仍然指向最新版。

set -euo pipefail

REPO="${QYZS_REPO:-qyzs-ai/qyzs-cli}"
GITHUB_URL="${QYZS_GITHUB_URL:-https://github.com/${REPO}}"
RELEASE_API_URL="${QYZS_RELEASE_API_URL:-https://api.github.com/repos/${REPO}/releases/latest}"
FALLBACK_TAG="${QYZS_FALLBACK_TAG:-v0.2.0}"

# >>> RELEASE_PINNED_VERSION (CI 在发布时替换为具体 tag; 仓库版本留空 = 装最新版)
PINNED_VERSION=""
# <<< RELEASE_PINNED_VERSION

# 计算文件的 sha256 (Linux 用 sha256sum, macOS 用 shasum -a 256)
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        printf ''
    fi
}

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

# 4. 解析要安装的版本
# 优先级: QYZS_VERSION > PINNED_VERSION (发布时钉定) > releases/latest 查询
TARGET_TAG="${QYZS_VERSION:-${PINNED_VERSION:-}}"
if [ -n "${TARGET_TAG}" ]; then
    echo "[INFO] 安装指定版本: ${TARGET_TAG}"
else
    # GitHub Release API 返回的是压缩成单行的 JSON, 且 release body 里同样含
    # "tag_name" 字样; 贪婪的 sed 会抓到行内最后一对引号 (release notes 末尾的
    # "**Full Changelog**: https://...")。故: 只取第一个匹配, 并校验其 tag 形态。
    while read -r candidate; do
        if printf '%s' "${candidate}" | grep -qE '^v?[0-9]+(\.[0-9]+){1,2}([-.+][A-Za-z0-9.]*)?$'; then
            TARGET_TAG="${candidate}"
            break
        fi
    done < <(curl -sSL "${RELEASE_API_URL}" \
        | grep -oE '"tag_name"[[:space:]]*:[[:space:]]*"[^"]+"' \
        | sed -E 's/.*"([^"]+)"$/\1/')
    if [ -z "${TARGET_TAG}" ]; then
        TARGET_TAG="${FALLBACK_TAG}"
    fi
    echo "[INFO] 准备下载版本: ${TARGET_TAG}"
fi

# 5. 下载并解压
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

ASSET_NAME="qyzs-${TARGET}.tar.gz"
DOWNLOAD_URL="${GITHUB_URL}/releases/download/${TARGET_TAG}/${ASSET_NAME}"
echo "[INFO] 下载地址: ${DOWNLOAD_URL}"

if curl -sSL -f "${DOWNLOAD_URL}" -o "${TMP_DIR}/${ASSET_NAME}"; then
    # 6. 完整性校验: 与 Release 资产的 SHA256SUMS 对照 (对不上即中止)
    SUMS_URL="${GITHUB_URL}/releases/download/${TARGET_TAG}/SHA256SUMS"
    EXPECTED=""
    if SUMS="$(curl -sSL -f "${SUMS_URL}" 2>/dev/null)"; then
        EXPECTED="$(printf '%s\n' "${SUMS}" | awk -v n="${ASSET_NAME}" '$2 == n || $2 == "*"n {print $1; exit}')"
    fi
    if [ -z "${EXPECTED}" ]; then
        echo "[WARN] 未取得 ${ASSET_NAME} 的 SHA256 期望值, 跳过完整性校验"
    else
        ACTUAL="$(sha256_of "${TMP_DIR}/${ASSET_NAME}")"
        if [ -z "${ACTUAL}" ]; then
            echo "[WARN] 本机无 sha256sum/shasum, 跳过完整性校验"
        elif [ "${ACTUAL}" != "${EXPECTED}" ]; then
            echo "[ERR] SHA256 校验不通过"
            echo "[ERR]   期望: ${EXPECTED}"
            echo "[ERR]   实际: ${ACTUAL}"
            echo "[ERR] 下载内容可能已被篡改, 已中止安装"
            exit 1
        else
            echo "[OK] SHA256 校验通过: ${ACTUAL:0:16}..."
        fi
    fi

    tar -xzf "${TMP_DIR}/${ASSET_NAME}" -C "${TMP_DIR}"
    chmod +x "${TMP_DIR}/qyzs"
    mv "${TMP_DIR}/qyzs" "${TARGET_BIN}"
else
    # CI 一律以压缩包形式发布资产（upload-rust-binary-action 的 archive: qyzs-<target>），
    # 不存在裸二进制资产。此处直接失败退出——否则下载失败仍会打印"安装成功"
    echo "[ERR] 下载失败: ${DOWNLOAD_URL}"
    echo "[ERR] 请确认该版本 (${TARGET_TAG}) 已发布, 且包含 ${TARGET} 平台资产"
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
