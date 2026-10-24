#!/usr/bin/env bash
# 把安装脚本的版本钉定到指定 tag (CI 发布时调用)
#
# 用法: bash tests/scripts/pin_install_scripts.sh <tag> [输出目录]
#   <tag>       完整 tag 名, **必须带 v 前缀** (如 v0.2.0) —— Release 资产的
#               下载路径用的就是带 v 的 tag, 去掉 v 会拼出 404 的 URL
#   [输出目录]   钉版后的脚本输出目录 (默认: 当前目录)
#
# 之所以做成独立脚本: CI 的发布流水线与本地发布验证脚本必须用**同一份**替换规则,
# 否则会出现「本地验证通过、CI 产出的却是错的」这类漂移 (曾发生: CI 侧把
# GITHUB_REF 的 v 前缀剥掉, 钉出的版本 0.2.0 与实际 tag v0.2.0 对不上)。

set -euo pipefail

TAG="${1:-}"
OUT_DIR="${2:-}"

if [ -z "${TAG}" ]; then
    echo "用法: $0 <tag> [输出目录]" >&2
    echo "  注意: tag 必须带 v 前缀 (如 v0.2.0), 否则 Release 下载 URL 会 404" >&2
    exit 64
fi

if ! printf '%s' "${TAG}" | grep -qE '^v[0-9]+(\.[0-9]+){1,2}([-.+][A-Za-z0-9.]*)?$'; then
    echo "[ERR] tag 形态不合法: ${TAG} (应为 vX.Y.Z 形式)" >&2
    exit 64
fi

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "${REPO_ROOT}"

if [ -n "${OUT_DIR}" ]; then
    mkdir -p "${OUT_DIR}"
    cp install.sh install.ps1 "${OUT_DIR}/"
    SH="${OUT_DIR}/install.sh"
    PS="${OUT_DIR}/install.ps1"
else
    SH="install.sh"
    PS="install.ps1"
fi

# 替换标记块内的版本行 (标记行本身保持不变, 便于重复执行)
sed -i "s|^PINNED_VERSION=.*$|PINNED_VERSION=\"${TAG}\"|" "${SH}"
sed -i "s|^\\\$PinnedVersion = .*$|\\\$PinnedVersion = \"${TAG}\"|" "${PS}"

# 自检: 替换必须生效, 且版本与传入 tag 完全一致
got_sh="$(grep -oE '^PINNED_VERSION="[^"]+"' "${SH}" | sed -E 's/.*"(.*)"/\1/')"
got_ps="$(grep -oE '^\$PinnedVersion = "[^"]+"' "${PS}" | sed -E 's/.*"(.*)"/\1/')"
if [ "${got_sh}" != "${TAG}" ] || [ "${got_ps}" != "${TAG}" ]; then
    echo "[ERR] 版本钉定失败: install.sh=${got_sh} install.ps1=${got_ps}, 期望 ${TAG}" >&2
    exit 1
fi

echo "[OK] 安装脚本已钉定到 ${TAG}"
