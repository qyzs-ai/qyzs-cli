#!/usr/bin/env python3
"""按 Release 的真实资产重新生成 winget / Homebrew 打包清单。

用法:
    python3 packaging/generate_manifests.py [--version v0.2.0] [--repo qyzs-ai/qyzs-cli]

行为:
    1. 取该 tag 的 SHA256SUMS (Release 资产), 读出各平台归档的哈希;
    2. 重写 packaging/winget/*.yaml 与 packaging/homebrew/qyzs.rb 里的
       版本号、下载地址与哈希;
    3. 校验生成结果: 三个 winget 文件的 PackageVersion 一致、哈希格式合法、
       Homebrew 的两个架构都有对应 sha256。

版本升级的完整流程见 packaging/README.md。
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
WINGET = ROOT / "packaging" / "winget"
BREW = ROOT / "packaging" / "homebrew"

WIN_ASSET = "qyzs-x86_64-pc-windows-msvc.zip"
MAC_ARM_ASSET = "qyzs-aarch64-apple-darwin.tar.gz"
MAC_X64_ASSET = "qyzs-x86_64-apple-darwin.tar.gz"

SHA_RE = re.compile(r"^[0-9a-f]{64}$")


def fetch_sums(repo: str, tag: str) -> dict[str, str]:
    """取该 tag 的 SHA256SUMS, 返回 {资产名: 哈希}"""
    import urllib.request

    url = f"https://github.com/{repo}/releases/download/{tag}/SHA256SUMS"
    print(f"  读取 {url}")
    with urllib.request.urlopen(url, timeout=60) as resp:
        text = resp.read().decode("utf-8")
    out: dict[str, str] = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) == 2 and SHA_RE.match(parts[0]):
            out[parts[1]] = parts[0]
    if not out:
        raise SystemExit("[ERR] SHA256SUMS 解析失败或为空")
    return out


def render_winget(version: str, win_sha: str) -> dict[str, str]:
    """重写三个 winget 文件, 返回 {文件名: 内容}"""
    ver = version.removeprefix("v")
    ident = "QYZS-AI.qyzs"

    version_yml = f"""PackageIdentifier: "{ident}"
PackageVersion: "{ver}"
DefaultLocale: "en-US"
ManifestType: "version"
ManifestVersion: "1.12.0"
"""

    installer_yml = f"""PackageIdentifier: "{ident}"
PackageVersion: "{ver}"
Platform:
  - "Windows.Desktop"
# Rust 1.78+ 的 MSVC 目标最低要求 Windows 10 (10.0.17763.0)
MinimumOSVersion: "10.0.17763.0"
InstallerType: "zip"
InstallModes:
  - "silent"
Installers:
  - Architecture: "x64"
    InstallerUrl: "https://github.com/qyzs-ai/qyzs-cli/releases/download/{version}/{WIN_ASSET}"
    InstallerSha256: "{win_sha}"
    # 便携 zip: winget 解压后按 RelativeFilePath 定位可执行文件, 并建 shim
    NestedInstallerType: "portable"
    NestedInstallerFiles:
      - RelativeFilePath: "qyzs.exe"
        PortableCommandAlias: "qyzs"
ManifestType: "installer"
ManifestVersion: "1.12.0"
"""

    locale_path = WINGET / f"{ident}.locale.en-US.yaml"
    locale = locale_path.read_text(encoding="utf-8")
    locale = re.sub(r'^PackageVersion: ".*"$', f'PackageVersion: "{ver}"', locale, flags=re.M)
    locale = re.sub(
        r'^ReleaseNotes: ".*"$',
        f'ReleaseNotes: "https://github.com/qyzs-ai/qyzs-cli/releases/tag/{version}"',
        locale,
        flags=re.M,
    )
    return {
        f"{ident}.yaml": version_yml,
        f"{ident}.installer.yaml": installer_yml,
        f"{ident}.locale.en-US.yaml": locale,
    }


def render_brew(version: str, arm_sha: str, x64_sha: str) -> str:
    ver = version.removeprefix("v")
    return f"""class Qyzs < Formula
  desc "Qingyang AI Computing Center unified CLI for large models and document pipelines"
  homepage "https://github.com/qyzs-ai/qyzs-cli"
  version "{ver}"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/qyzs-ai/qyzs-cli/releases/download/{version}/{MAC_ARM_ASSET}"
      sha256 "{arm_sha}"
    else
      url "https://github.com/qyzs-ai/qyzs-cli/releases/download/{version}/{MAC_X64_ASSET}"
      sha256 "{x64_sha}"
    end
  end

  def install
    bin.install "qyzs"
  end

  test do
    assert_match "qyzs", shell_output("#{{bin}}/qyzs --version")
  end
end
"""


def validate(files: dict[str, str], brew: str) -> list[str]:
    """返回问题列表 (空 = 全部通过)"""
    problems: list[str] = []
    versions = set()
    for name, content in files.items():
        m = re.search(r'^PackageVersion: "(.+)"$', content, re.M)
        if not m:
            problems.append(f"{name}: 缺 PackageVersion")
            continue
        versions.add(m.group(1))
        m2 = re.search(r'^PackageIdentifier: "(.+)"$', content, re.M)
        if not m2:
            problems.append(f"{name}: 缺 PackageIdentifier")
    if len(versions) > 1:
        problems.append(f"三个 winget 文件的 PackageVersion 不一致: {versions}")
    for name, content in files.items():
        for sha in re.findall(r'InstallerSha256: "([0-9a-f]+)"', content):
            if not SHA_RE.match(sha):
                problems.append(f"{name}: InstallerSha256 格式非法: {sha}")
    for arch, sha in re.findall(
        r'url ".*?/(qyzs-(?:aarch64|x86_64)-apple-darwin\.tar\.gz)"\n\s+sha256 "([0-9a-f]+)"', brew
    ):
        if not SHA_RE.match(sha):
            problems.append(f"homebrew: {arch} 的 sha256 格式非法: {sha}")
    if brew.count("sha256") != 2:
        problems.append(f"homebrew: 应有两个架构的 sha256, 实际 {brew.count('sha256')} 个")
    if 'bin.install "qyzs"' not in brew:
        problems.append("homebrew: 缺 bin.install")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", default="v0.2.0", help="Release tag (如 v0.2.0)")
    ap.add_argument("--repo", default="qyzs-ai/qyzs-cli")
    ap.add_argument("--check", action="store_true", help="只校验现有文件, 不重写")
    args = ap.parse_args()

    print(f"== 校验现有清单 ({args.version}) ==")
    if args.check:
        ident = "QYZS-AI.qyzs"
        files = {
            p.name: p.read_text(encoding="utf-8")
            for p in sorted(WINGET.glob(f"{ident}*.yaml"))
        }
        brew = (BREW / "qyzs.rb").read_text(encoding="utf-8")
        problems = validate(files, brew)
        # 与线上 Release 的真实哈希逐一比对 —— 只查格式查不出"抄错哈希"
        try:
            sums = fetch_sums(args.repo, args.version)
            for need, expect in (
                (WIN_ASSET, files[f"{ident}.installer.yaml"]),
                (MAC_ARM_ASSET, brew),
                (MAC_X64_ASSET, brew),
            ):
                if need not in sums:
                    problems.append(f"{need}: Release 的 SHA256SUMS 中缺失")
                    continue
                if sums[need] not in expect:
                    problems.append(
                        f"{need}: 清单中的哈希与 Release 不一致 (线上 {sums[need][:16]}...)"
                    )
        except Exception as e:  # 网络不可用等: 只提示, 不掩盖格式问题
            print(f"  [WARN] 无法取线上 SHA256SUMS 比对: {e}")
        for p in problems:
            print(f"  [FAIL] {p}")
        print(f"  → {'全部通过' if not problems else f'{len(problems)} 处问题'}")
        return 1 if problems else 0

    print(f"== 取 {args.repo} {args.version} 的 SHA256SUMS ==")
    sums = fetch_sums(args.repo, args.version)
    for need in (WIN_ASSET, MAC_ARM_ASSET, MAC_X64_ASSET):
        if need not in sums:
            raise SystemExit(f"[ERR] SHA256SUMS 缺少资产 {need}")
        print(f"  ✓ {need}  {sums[need][:16]}...")

    print("== 生成清单 ==")
    files = render_winget(args.version, sums[WIN_ASSET])
    for name, content in files.items():
        (WINGET / name).write_text(content, encoding="utf-8")
        print(f"  写入 packaging/winget/{name}")
    brew = render_brew(args.version, sums[MAC_ARM_ASSET], sums[MAC_X64_ASSET])
    (BREW / "qyzs.rb").write_text(brew, encoding="utf-8")
    print("  写入 packaging/homebrew/qyzs.rb")

    problems = validate(files, brew)
    for p in problems:
        print(f"  [FAIL] {p}")
    print(f"  → {'校验通过' if not problems else f'{len(problems)} 处问题'}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
