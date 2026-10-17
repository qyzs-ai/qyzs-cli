#!/usr/bin/env bash
# 装配 npm 平台包族结构

set -euo pipefail

VERSION="${1:-0.1.0}"
VERSION="${VERSION#v}" # 剥除前导 v

DIST_DIR="dist/npm"
rm -rf "${DIST_DIR}"
mkdir -p "${DIST_DIR}/cli/bin"

# 1. 复制顶层 cli 包
cp npm/cli/package.json "${DIST_DIR}/cli/package.json"
sed -i "s/\"version\": \".*\"/\"version\": \"${VERSION}\"/g" "${DIST_DIR}/cli/package.json"
# optionalDependencies 的平台包版本钉必须与顶层版本同步，否则新版顶层包会拉到旧版平台二进制
sed -i "s|\"\(@qyzs-ai/cli-[a-z0-9-]*\)\": \"[^\"]*\"|\"\1\": \"${VERSION}\"|g" "${DIST_DIR}/cli/package.json"
cp npm/cli/bin/qyzs.js "${DIST_DIR}/cli/bin/qyzs.js"
chmod +x "${DIST_DIR}/cli/bin/qyzs.js"

# 2. 生成各平台子包
PLATFORMS=(
  "linux-x64:linux:x64:qyzs"
  "linux-arm64:linux:arm64:qyzs"
  "linux-loong64:linux:loong64:qyzs"
  "darwin-x64:darwin:x64:qyzs"
  "darwin-arm64:darwin:arm64:qyzs"
  "win32-x64:win32:x64:qyzs.exe"
)

for item in "${PLATFORMS[@]}"; do
  IFS=":" read -r PKG_NAME OS ARCH BIN_NAME <<< "${item}"
  PKG_DIR="${DIST_DIR}/${PKG_NAME}"
  mkdir -p "${PKG_DIR}/bin"

  cat <<EOF > "${PKG_DIR}/package.json"
{
  "name": "@qyzs-ai/cli-${PKG_NAME}",
  "version": "${VERSION}",
  "description": "Prebuilt qyzs CLI binary for ${OS} ${ARCH}",
  "os": ["${OS}"],
  "cpu": ["${ARCH}"],
  "license": "Apache-2.0",
  "files": ["bin/${BIN_NAME}"]
}
EOF

  # 若 dist/ 下存在对应构建二进制，复制就位
  if [ -f "dist/${BIN_NAME}-${PKG_NAME}" ]; then
    cp "dist/${BIN_NAME}-${PKG_NAME}" "${PKG_DIR}/bin/${BIN_NAME}"
    chmod +x "${PKG_DIR}/bin/${BIN_NAME}"
  fi
done

echo "[OK] npm 包族装配就绪: ${DIST_DIR}"
