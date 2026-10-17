#!/usr/bin/env node
/**
 * @qyzs-ai/cli 原生平台自适应调度器
 */

const { spawnSync } = require("child_process");
const path = require("path");
const fs = require("fs");

function getPlatformPackageName() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "linux") {
    if (arch === "x64") return "@qyzs-ai/cli-linux-x64";
    if (arch === "arm64") return "@qyzs-ai/cli-linux-arm64";
    if (arch === "loong64") return "@qyzs-ai/cli-linux-loong64";
  } else if (platform === "darwin") {
    if (arch === "x64") return "@qyzs-ai/cli-darwin-x64";
    if (arch === "arm64") return "@qyzs-ai/cli-darwin-arm64";
  } else if (platform === "win32") {
    if (arch === "x64") return "@qyzs-ai/cli-win32-x64";
  }

  throw new Error(`当前系统环境不支持: ${platform} ${arch}`);
}

function resolveBinaryPath() {
  const pkgName = getPlatformPackageName();
  const exeName = process.platform === "win32" ? "qyzs.exe" : "qyzs";

  try {
    const pkgJsonPath = require.resolve(`${pkgName}/package.json`);
    const binPath = path.join(path.dirname(pkgJsonPath), "bin", exeName);
    if (fs.existsSync(binPath)) {
      return binPath;
    }
  } catch (err) {
    // 依赖未能通过 optionalDependencies 找到，尝试在 PATH 中查找
  }

  // 回退探测同目录或全局
  const fallback = path.join(__dirname, exeName);
  if (fs.existsSync(fallback)) {
    return fallback;
  }

  throw new Error(
    `未找到匹配当前系统的二进制包 (${pkgName})。\n请尝试重新执行: npm install -g @qyzs-ai/cli`
  );
}

function main() {
  const binPath = resolveBinaryPath();
  const args = process.argv.slice(2);

  const result = spawnSync(binPath, args, {
    stdio: "inherit",
    windowsHide: true,
  });

  if (result.error) {
    console.error(`[ERR] 执行失败: ${result.error.message}`);
    process.exit(1);
  }

  process.exit(result.status ?? 0);
}

main();
